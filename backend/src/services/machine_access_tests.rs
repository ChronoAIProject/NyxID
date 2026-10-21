use super::{
    assistant_authority_tests::{Fixture, fixture, orchestrator_fixture},
    machine_access_service as access,
};
use crate::{
    errors::AppError,
    models::{
        assistant_agent::{AssistantAgent, COLLECTION_NAME as AGENTS},
        machine_access::{self as model, Selection},
    },
};
use mongodb::bson::{self, Document, doc};
use nyxid_machine::{Operation, authority::Capabilities};
use serde_json::json;
async fn node(f: &Fixture, owner: &str) -> crate::models::node::Node {
    let name = format!("machine-{}", uuid::Uuid::new_v4());
    // These authority fixtures model an owner's explicit legacy setup choice.
    // W9.4 missing-choice tests clear it deliberately below.
    let mut node = Box::pin(super::machine_integration_tests::ungranted_node_named(
        f, owner, &name,
    ))
    .await;
    node.machine.as_mut().unwrap().separation_choice =
        Some(nyxid_machine::context::Choice::SharedLegacy);
    f.state
        .db
        .collection::<Document>(crate::models::node::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&node.id},
            doc! {"$set":{"machine":bson::to_bson(&node.machine).unwrap()}},
        )
        .await
        .unwrap();
    node
}
async fn enable(f: &Fixture) {
    super::security_readiness_service::ready_test_replica(&f.state.db).await;
    access::ensure_indexes(&f.state.db).await.unwrap();
    super::feature_flag_service::set_platform_override(
        &f.state.db,
        access::FLAG,
        &super::feature_flag_service::FlagTarget::Global,
        true,
        &f.owner,
    )
    .await
    .unwrap();
}
async fn v2(f: &Fixture) -> crate::models::node::Node {
    let mut node = node(f, &f.owner).await;
    node.machine.as_mut().unwrap().authority_versions = vec![2];
    node.machine.as_mut().unwrap().authority_timing_v1 = true;
    f.state
        .db
        .collection::<Document>(crate::models::node::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&node.id},
            doc! {"$set":{"machine":bson::to_bson(&node.machine).unwrap()}},
        )
        .await
        .unwrap();
    node
}
fn selection(revision: i64, caps: Capabilities) -> Selection {
    Selection {
        mode: None,
        expected_revision: revision,
        capabilities: caps,
        saved_login_ids: None,
    }
}
#[tokio::test]
async fn machine_access_cutover_materializes_legacy_nyxbot_and_does_not_inherit_new_nodes() {
    let f = orchestrator_fixture("machine_access_cutover").await;
    let old = node(&f, &f.owner).await;
    // A persisted pre-upgrade NyxBot has no policy, just its implicit reachability.
    f.state
        .db
        .collection::<Document>(AGENTS)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$unset":{"machine_access":""}},
        )
        .await
        .unwrap();
    access::ensure_indexes(&f.state.db).await.unwrap();
    let policy = Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
        .await
        .unwrap();
    assert!(policy.assignments[&old.id].legacy);
    assert!(policy.assignments[&old.id].capabilities.shell);
    let new = node(&f, &f.owner).await;
    assert!(
        !Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
            .await
            .unwrap()
            .assignments
            .contains_key(&new.id)
    );
    assert!(
        Box::pin(access::authorize(
            &f.state.db,
            &f.chat,
            &old,
            Operation::Exec,
            &json!({})
        ))
        .await
        .is_ok()
    );
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn machine_access_legacy_roster_beyond_editor_page_keeps_every_assignment() {
    let f = orchestrator_fixture("machine_access_large_legacy").await;
    let template = node(&f, &f.owner).await;
    let mut rows = Vec::new();
    for _ in 0..501 {
        let mut row = template.clone();
        row.id = uuid::Uuid::new_v4().to_string();
        row.name = row.id.clone();
        rows.push(row);
    }
    let last = rows.last().unwrap().clone();
    f.state
        .db
        .collection::<crate::models::node::Node>(crate::models::node::COLLECTION_NAME)
        .insert_many(rows)
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(AGENTS)
        .update_one(
            doc! {"_id": &f.chat.agent_id},
            doc! {"$unset": {"machine_access": ""}},
        )
        .await
        .unwrap();
    access::ensure_indexes(&f.state.db).await.unwrap();
    let policy = Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
        .await
        .unwrap();
    assert_eq!(policy.assignments.len(), 502);
    assert!(policy.assignments.values().all(|a| a.legacy));
    assert!(
        Box::pin(access::authorize(
            &f.state.db,
            &f.chat,
            &last,
            Operation::Exec,
            &json!({})
        ))
        .await
        .is_ok()
    );
    assert_eq!(
        Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
            .await
            .unwrap(),
        policy
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_delayed_cutover_does_not_inherit_new_org_membership() {
    use crate::{
        models::{org_membership::OrgRole, user::UserType},
        test_utils::{test_membership, test_user},
    };
    let f = orchestrator_fixture("machine_access_late_membership").await;
    let org = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    let old_node = node(&f, &org).await;
    f.state
        .db
        .collection::<Document>(AGENTS)
        .update_one(
            doc! {"_id": &f.chat.agent_id},
            doc! {"$unset": {"machine_access": ""}},
        )
        .await
        .unwrap();
    access::ensure_indexes(&f.state.db).await.unwrap();
    let mut membership = test_membership(&org, &f.owner, OrgRole::Admin, None);
    membership.created_at = chrono::Utc::now() + chrono::Duration::seconds(1);
    f.state
        .db
        .collection(crate::models::org_membership::COLLECTION_NAME)
        .insert_one(membership)
        .await
        .unwrap();
    let policy = Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
        .await
        .unwrap();
    assert!(!policy.assignments.contains_key(&old_node.id));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_new_membership_is_deny_by_default_and_old_writes_preserve_limits() {
    let f = fixture("machine_access_default").await;
    enable(&f).await;
    let n = v2(&f).await;
    let change = super::assistant_team_service::GrantChange::Machine {
        base: Box::new(super::assistant_team_service::GrantChange::Add(
            Default::default(),
        )),
        machines: Some(vec![n.id.clone()]),
        logins: None,
        mode: super::assistant_team_service::MachineGrantMode::Add,
    };
    Box::pin(super::assistant_team_service::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        change.clone(),
    ))
    .await
    .unwrap();
    assert!(matches!(
        Box::pin(access::authorize(
            &f.state.db,
            &f.chat,
            &n,
            Operation::Exec,
            &json!({})
        ))
        .await,
        Err(AppError::MachinePermissionRevoked)
    ));
    let revision = Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
        .await
        .unwrap()
        .revision;
    let configured = Box::pin(access::configure(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &n.id,
        selection(
            revision,
            Capabilities {
                files: true,
                ..Default::default()
            },
        ),
    ))
    .await
    .unwrap();
    Box::pin(super::assistant_team_service::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        change,
    ))
    .await
    .unwrap();
    assert_eq!(
        Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
            .await
            .unwrap(),
        configured
    );
    assert!(
        Box::pin(access::authorize(
            &f.state.db,
            &f.chat,
            &n,
            Operation::ReadFile,
            &json!({})
        ))
        .await
        .is_ok()
    );
    assert!(
        Box::pin(access::authorize(
            &f.state.db,
            &f.chat,
            &n,
            Operation::Exec,
            &json!({})
        ))
        .await
        .is_err()
    );
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn machine_access_new_specialist_machine_selection_stores_explicit_denial() {
    let f = orchestrator_fixture("machine_access_new_specialist").await;
    enable(&f).await;
    let n = v2(&f).await;
    let (agent, _) = Box::pin(super::assistant_team_service::create_specialist(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        super::assistant_team_service::CreateRequest {
            machines: Some(vec![n.id.clone()]),
            logins: None,
            name: "new-worker".into(),
            description: "Work on the selected machine".into(),
            display_name: None,
            persona: None,
            targets: Default::default(),
            account_read: false,
            specialty: None,
            created_by: "user",
        },
    ))
    .await
    .unwrap()
    .unwrap();
    let p = Box::pin(access::policy(&f.state.db, &agent.id))
        .await
        .unwrap();
    assert_eq!(p.assignments[&n.id].capabilities, Capabilities::default());
    assert!(!p.assignments[&n.id].legacy);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_enforcement_ignores_flag_and_revocation_is_transactional() {
    let f = orchestrator_fixture("machine_access_revoke").await;
    enable(&f).await;
    let n = v2(&f).await;
    Box::pin(access::configure(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &n.id,
        selection(
            1,
            Capabilities {
                shell: true,
                ..Default::default()
            },
        ),
    ))
    .await
    .unwrap();
    let a = Box::pin(access::admit(
        &f.state.db,
        &f.chat,
        &n,
        Operation::Exec,
        &json!({}),
        None,
    ))
    .await
    .unwrap()
    .unwrap();
    let b = Box::pin(access::configure(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &n.id,
        selection(a.revision, Capabilities::default()),
    ))
    .await
    .unwrap();
    assert!(b.revision > a.revision);
    assert!(
        f.state
            .db
            .collection::<Document>(model::OUTBOX)
            .find_one(doc! {"agent_id":&f.chat.agent_id,"revision":b.revision,"pending":true})
            .await
            .unwrap()
            .is_some()
    );
    super::feature_flag_service::set_platform_override(
        &f.state.db,
        access::FLAG,
        &super::feature_flag_service::FlagTarget::Global,
        false,
        &f.owner,
    )
    .await
    .unwrap();
    assert!(
        Box::pin(access::authorize(
            &f.state.db,
            &f.chat,
            &n,
            Operation::Exec,
            &json!({})
        ))
        .await
        .is_err()
    );
    let raw = f
        .state
        .db
        .collection::<Document>(model::LEASES)
        .find_one(doc! {"_id":&a.lease_id})
        .await
        .unwrap()
        .unwrap();
    assert!(!raw.to_string().contains("command"));
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn machine_access_acl_and_old_nodes_fail_closed() {
    let f = orchestrator_fixture("machine_access_acl").await;
    enable(&f).await;
    let old = node(&f, &f.owner).await;
    assert!(matches!(
        Box::pin(access::configure(
            &f.state.db,
            &f.owner,
            &f.chat.agent_id,
            &old.id,
            selection(1, Default::default())
        ))
        .await,
        Err(AppError::MachineAuthorityUnsupported)
    ));
    let n = v2(&f).await;
    assert!(
        Box::pin(access::configure(
            &f.state.db,
            &uuid::Uuid::new_v4().to_string(),
            &f.chat.agent_id,
            &n.id,
            selection(1, Default::default())
        ))
        .await
        .is_err()
    );
    let mut guest = f.chat.clone();
    guest.guest = true;
    assert!(
        Box::pin(access::assignment(&f.state.db, &guest, &n))
            .await
            .is_err()
    );
    assert!(
        Box::pin(access::configure(
            &f.state.db,
            &f.owner,
            &f.chat.agent_id,
            &n.id,
            selection(
                1,
                Capabilities {
                    computer: true,
                    ..Default::default()
                }
            )
        ))
        .await
        .is_err()
    );
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn machine_access_contexts_partition_people_and_groups_and_tool_discovery() {
    let f = orchestrator_fixture("machine_access_contexts").await;
    enable(&f).await;
    let n = v2(&f).await;
    Box::pin(access::configure(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &n.id,
        selection(
            1,
            Capabilities {
                browser: true,
                ..Default::default()
            },
        ),
    ))
    .await
    .unwrap();
    let a = Box::pin(access::admit(
        &f.state.db,
        &f.chat,
        &n,
        Operation::Browser,
        &json!({}),
        None,
    ))
    .await
    .unwrap()
    .unwrap();
    let b = Box::pin(access::admit(
        &f.state.db,
        &f.chat,
        &n,
        Operation::Browser,
        &json!({}),
        None,
    ))
    .await
    .unwrap()
    .unwrap();
    assert_eq!(a.context_id, b.context_id);
    assert_eq!(
        uuid::Uuid::parse_str(&a.context_id)
            .unwrap()
            .get_version_num(),
        4
    );
    assert_ne!(a.lease_id, b.lease_id);
    assert_eq!(a.actor_id, f.owner);
    // Group tasks for the same agent get distinct signed metadata contexts.
    let mut ids = vec![a.context_id.clone()];
    for name in ["Group A", "Group B"] {
        let group = super::assistant_group_service::create(
            &f.state.db,
            &f.owner,
            name,
            std::slice::from_ref(&f.chat.agent_id),
            "user",
        )
        .await
        .unwrap();
        f.state
            .db
            .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&f.chat.conversation_id},
                doc! {"$set":{"group_id":&group.id}},
            )
            .await
            .unwrap();
        let grouped = Box::pin(access::admit(
            &f.state.db,
            &f.chat,
            &n,
            Operation::Browser,
            &json!({}),
            None,
        ))
        .await
        .unwrap()
        .unwrap();
        assert_eq!(grouped.group_id.as_deref(), Some(group.id.as_str()));
        assert!(grouped.valid(
            &n.machine.as_ref().unwrap().runtime_id,
            chrono::Utc::now().timestamp_millis()
        ));
        assert!(!ids.contains(&grouped.context_id));
        ids.push(grouped.context_id);
    }

    let definitions = Box::pin(access::definitions(&f.state.db, &f.chat))
        .await
        .unwrap();
    assert!(definitions.iter().any(|t| t.name == "nyx__machine_browser"));
    assert!(!definitions.iter().any(|t| t.name == "nyx__machine_exec"));
    assert!(
        Box::pin(access::authorize(
            &f.state.db,
            &f.chat,
            &n,
            Operation::Browser,
            &json!({"browser":"dev"})
        ))
        .await
        .is_err()
    );
    assert!(
        f.state
            .db
            .collection::<AssistantAgent>(AGENTS)
            .find_one(doc! {"_id":&f.chat.agent_id})
            .await
            .unwrap()
            .unwrap()
            .machine_access
            .is_some()
    );
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn machine_access_native_widening_requires_owner_card() {
    let f = orchestrator_fixture("machine_access_card").await;
    enable(&f).await;
    let n = v2(&f).await;
    let (listing, pending) = Box::pin(crate::handlers::machine_access::native(
        &f.state,
        &f.chat,
        &json!({}),
    ))
    .await
    .unwrap();
    assert!(!pending);
    assert_eq!(listing["machines"][0]["node_id"], n.id);
    let args = json!({"agent":f.chat.agent_id,"machine":n.id,"selection":selection(1,Capabilities{files:true,..Default::default()})});
    let (result, pending) = Box::pin(crate::handlers::machine_access::native(
        &f.state, &f.chat, &args,
    ))
    .await
    .unwrap();
    assert!(pending);
    assert!(result["acknowledgement_id"].is_string());
    assert!(
        Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
            .await
            .unwrap()
            .assignments
            .is_empty()
    );
    let id = result["acknowledgement_id"].as_str().unwrap();
    super::assistant_acknowledgement_service::decide(
        &f.state.db,
        &f.owner,
        &f.chat.conversation_id,
        id,
        true,
    )
    .await
    .unwrap();
    let mut approved = args.clone();
    approved["acknowledgement_id"] = id.into();
    let (outcome, pending) = Box::pin(crate::handlers::machine_access::native(
        &f.state, &f.chat, &approved,
    ))
    .await
    .unwrap();
    assert!(!pending);
    assert_eq!(outcome["revision"], 2);
    assert!(
        Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
            .await
            .unwrap()
            .assignments[&n.id]
            .capabilities
            .files
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_revision_conflicts_and_human_route_guard() {
    let f = orchestrator_fixture("machine_access_human").await;
    enable(&f).await;
    let n = v2(&f).await;
    let first = Box::pin(access::configure(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &n.id,
        selection(
            1,
            Capabilities {
                files: true,
                ..Default::default()
            },
        ),
    ))
    .await
    .unwrap();
    assert!(matches!(
        Box::pin(access::configure(
            &f.state.db,
            &f.owner,
            &f.chat.agent_id,
            &n.id,
            selection(
                1,
                Capabilities {
                    shell: true,
                    ..Default::default()
                }
            )
        ))
        .await,
        Err(AppError::Conflict(_))
    ));
    let denied = Box::pin(crate::handlers::machine_access::get(
        axum::extract::State(f.state.clone()),
        f.auth.clone(),
        axum::extract::Path(f.chat.agent_id.clone()),
    ))
    .await;
    assert!(
        denied.is_err(),
        "agent key must never become a human grant editor"
    );
    let human = crate::test_utils::test_auth_user(&f.owner);
    let result = Box::pin(crate::handlers::machine_access::get(
        axum::extract::State(f.state.clone()),
        human,
        axum::extract::Path(f.chat.agent_id.clone()),
    ))
    .await
    .unwrap();
    let dto = serde_json::to_value(result.0).unwrap();
    assert_eq!(dto[0]["name"], n.name);
    assert_eq!(dto[0]["revision"], first.revision);
    assert_eq!(dto[0]["protocol_v2"], true);
    assert_eq!(dto[0]["can_edit"], true);
    assert_eq!(dto[0]["revocation_pending"], true);
    assert!(dto[0].get("user_id").is_none());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_destroy_is_atomic_with_outbox_and_idempotent_delivery() {
    let f = fixture("machine_access_destroy").await;
    enable(&f).await;
    let n = v2(&f).await;
    Box::pin(access::configure(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &n.id,
        selection(
            1,
            Capabilities {
                shell: true,
                ..Default::default()
            },
        ),
    ))
    .await
    .unwrap();
    let destroyed = Box::pin(super::assistant_team_service::destroy(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
    ))
    .await
    .unwrap();
    let revision = destroyed.machine_access.unwrap().revision;
    assert!(
        Box::pin(access::assignment(&f.state.db, &f.chat, &n))
            .await
            .is_err()
    );
    let outbox = f.state.db.collection::<Document>(model::OUTBOX);
    let row = outbox
        .find_one(doc! {"agent_id":&f.chat.agent_id,"revision":revision,"pending":true})
        .await
        .unwrap()
        .unwrap();
    let (task, mut requests) =
        super::machine_integration_tests::peer(&f, &n, json!({"accepted":true})).await;
    let (one, two) = tokio::join!(
        Box::pin(access::revoke_one(&f.state, row.clone())),
        Box::pin(access::revoke_one(&f.state, row.clone()))
    );
    one.unwrap();
    two.unwrap();
    let request = requests.recv().await.unwrap();
    assert_eq!(request.operation, Operation::AuthorityRevoke);
    assert_eq!(request.version, 2);
    assert_eq!(request.authority.unwrap().revision, revision);
    assert!(
        requests.try_recv().is_err(),
        "only the outbox claim holder sends cancellation"
    );
    assert!(
        !outbox
            .find_one(doc! {"_id":row.get_str("_id").unwrap()})
            .await
            .unwrap()
            .unwrap()
            .get_bool("pending")
            .unwrap()
    );
    Box::pin(access::revoke_one(&f.state, row)).await.unwrap();
    assert!(requests.try_recv().is_err());
    task.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_org_contexts_partition_people_and_recheck_membership() {
    use super::{
        assistant_acknowledgement_service as acks, assistant_nyxagent as engine,
        assistant_team_service as team, org_agent_tests::Fixture as OrgFixture,
    };
    use uuid::Uuid;
    let f = OrgFixture::new("machine_access_people").await;
    let (agent, _) = f.create().await;
    access::ensure_indexes(&f.state.db).await.unwrap();
    super::feature_flag_service::set_platform_override(
        &f.state.db,
        access::FLAG,
        &super::feature_flag_service::FlagTarget::Global,
        true,
        &f.admin,
    )
    .await
    .unwrap();
    let (_, token, _) = super::node_service::create_registration_token_for_actor(
        &f.state.db,
        &f.admin,
        &f.org,
        "org-machine",
        100,
        300,
    )
    .await
    .unwrap();
    let (mut n, _, _) =
        super::node_service::register_node(&f.state.db, &f.state.encryption_keys, &token, None)
            .await
            .unwrap();
    n.machine = Some(nyxid_machine::MachineProfile {
        version: 1,
        authority_versions: vec![2],
        runtime_id: Uuid::new_v4().to_string(),
        files: true,
        separation_choice: Some(nyxid_machine::context::Choice::SharedLegacy),
        ..Default::default()
    });
    f.state
        .db
        .collection::<crate::models::node::Node>(crate::models::node::COLLECTION_NAME)
        .replace_one(doc! {"_id":&n.id}, &n)
        .await
        .unwrap();
    Box::pin(access::configure(
        &f.state.db,
        &f.admin,
        &agent.id,
        &n.id,
        selection(
            1,
            Capabilities {
                files: true,
                ..Default::default()
            },
        ),
    ))
    .await
    .unwrap();
    assert!(
        Box::pin(access::configure(
            &f.state.db,
            &f.member,
            &agent.id,
            &n.id,
            selection(2, Capabilities::default())
        ))
        .await
        .is_err(),
        "machine access editing requires owner write ACL"
    );
    let mut authorities = Vec::new();
    let mut chats = Vec::new();
    for actor in [&f.admin, &f.member] {
        let row = Box::pin(team::home_thread_for(
            &f.state.db,
            &f.state.encryption_keys,
            actor,
            &agent,
        ))
        .await
        .unwrap();
        Box::pin(engine::begin_turn(
            &f.state.db,
            actor,
            &engine::TurnRequest {
                acknowledgement_id: None,
                retry_of: None,
                conversation_id: Some(row.id.clone()),
                text: "Inspect workspace".into(),
                attachment_ids: vec![],
                agent_id: None,
                model: None,
                access_mode: None,
            },
            &f.state.encryption_keys,
        ))
        .await
        .unwrap();
        let chat = acks::for_key(&f.state.db, actor, Some(&row.credential_api_key_id))
            .await
            .unwrap()
            .unwrap();
        authorities.push(
            Box::pin(access::admit(
                &f.state.db,
                &chat,
                &n,
                Operation::ReadFile,
                &json!({}),
                None,
            ))
            .await
            .unwrap()
            .unwrap(),
        );
        chats.push(chat);
    }
    assert_ne!(authorities[0].context_id, authorities[1].context_id);
    assert_eq!(authorities[1].actor_id, f.member);
    assert_eq!(authorities[1].owner_id, f.org);
    f.revoke(&f.member).await;
    assert!(
        Box::pin(access::admit(
            &f.state.db,
            &chats[1],
            &n,
            Operation::ReadFile,
            &json!({}),
            None
        ))
        .await
        .is_err()
    );
    assert!(
        Box::pin(access::admit(
            &f.state.db,
            &chats[0],
            &n,
            Operation::ReadFile,
            &json!({}),
            None
        ))
        .await
        .is_ok()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_assignment_revision_keeps_other_machines_live() {
    let f = orchestrator_fixture("machine_access_scoped_revision").await;
    enable(&f).await;
    let a = v2(&f).await;
    let b = v2(&f).await;
    let caps = Capabilities {
        shell: true,
        ..Default::default()
    };
    let p = Box::pin(access::configure(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &a.id,
        selection(1, caps),
    ))
    .await
    .unwrap();
    Box::pin(access::configure(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &b.id,
        selection(p.revision, caps),
    ))
    .await
    .unwrap();
    let first = Box::pin(access::admit(
        &f.state.db,
        &f.chat,
        &a,
        Operation::Exec,
        &json!({}),
        None,
    ))
    .await
    .unwrap()
    .unwrap();
    let p = Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
        .await
        .unwrap();
    Box::pin(access::configure(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &b.id,
        selection(p.revision, Capabilities::default()),
    ))
    .await
    .unwrap();
    let second = Box::pin(access::admit(
        &f.state.db,
        &f.chat,
        &a,
        Operation::Exec,
        &json!({}),
        None,
    ))
    .await
    .unwrap()
    .unwrap();
    assert_eq!(first.revision, second.revision);
    assert!(
        Box::pin(access::authorize(
            &f.state.db,
            &f.chat,
            &b,
            Operation::Exec,
            &json!({})
        ))
        .await
        .is_err()
    );
    let current = Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
        .await
        .unwrap();
    assert!(
        f.state
            .db
            .collection::<Document>(model::OUTBOX)
            .find_one(doc! {"node_id":&a.id,"revision":current.revision})
            .await
            .unwrap()
            .is_none()
    );
    let job = super::machine_service::issue_job(&f.state.db, &f.chat, &a, 120, vec![])
        .await
        .unwrap();
    let cancellation = Box::pin(access::admit(
        &f.state.db,
        &f.chat,
        &a,
        Operation::JobCancel,
        &json!({"job_id":job.id}),
        None,
    ))
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        cancellation.revision, first.revision,
        "cancellation must not raise an unrelated assignment's fence"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_legacy_revocation_resumes_bounded_batches_before_acknowledgement() {
    let f = fixture("machine_access_legacy_revoke").await;
    enable(&f).await;
    let n = node(&f, &f.owner).await;
    let conversations = f.state.db.collection::<Document>("assistant_conversations");
    for _ in 0..22 {
        conversations
            .insert_one(
                doc! {"_id": uuid::Uuid::new_v4().to_string(), "agent_id": &f.chat.agent_id},
            )
            .await
            .unwrap();
    }
    let outbox = f.state.db.collection::<Document>(model::OUTBOX);
    let id = uuid::Uuid::new_v4().to_string();
    outbox.insert_one(doc! {"_id": &id, "node_id": &n.id, "agent_id": &f.chat.agent_id, "revision": 2_i64, "pending": true, "claim_until": bson::DateTime::from_millis(0)}).await.unwrap();
    let row = outbox.find_one(doc! {"_id": &id}).await.unwrap().unwrap();
    let (task, mut requests) =
        super::machine_integration_tests::peer(&f, &n, json!({"cancelled":true})).await;
    Box::pin(access::revoke_one(&f.state, row.clone()))
        .await
        .unwrap();
    let progressed = outbox.find_one(doc! {"_id": &id}).await.unwrap().unwrap();
    assert!(progressed.get_bool("pending").unwrap());
    assert!(progressed.get_str("conversation_cursor").is_ok());
    // A stale stream/sweep snapshot must take the current cursor from its claim.
    Box::pin(access::revoke_one(&f.state, row.clone()))
        .await
        .unwrap();
    assert!(
        !outbox
            .find_one(doc! {"_id": &id})
            .await
            .unwrap()
            .unwrap()
            .get_bool("pending")
            .unwrap()
    );
    let mut seen = std::collections::HashSet::new();
    while let Ok(request) = requests.try_recv() {
        assert_eq!(request.operation, Operation::Cancel);
        assert_eq!(request.version, 1);
        assert!(
            seen.insert(
                request.parameters["conversation_id"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            )
        );
    }
    assert_eq!(seen.len(), 23);
    Box::pin(access::revoke_one(&f.state, row)).await.unwrap();
    assert!(requests.try_recv().is_err());
    task.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_renewal_uses_live_grants_and_turn_state() {
    let f = orchestrator_fixture("machine_access_renewal").await;
    enable(&f).await;
    let n = v2(&f).await;
    Box::pin(access::configure(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &n.id,
        selection(
            1,
            Capabilities {
                shell: true,
                ..Default::default()
            },
        ),
    ))
    .await
    .unwrap();
    let authority = Box::pin(access::admit(
        &f.state.db,
        &f.chat,
        &n,
        Operation::Exec,
        &json!({}),
        None,
    ))
    .await
    .unwrap()
    .unwrap();
    let leases = f.state.db.collection::<model::Lease>(model::LEASES);
    let mut lease = leases
        .find_one(doc! {"_id":&authority.lease_id})
        .await
        .unwrap()
        .unwrap();
    lease.authority.expires_at_ms -= 1_000;
    let (task, mut requests) =
        super::machine_integration_tests::peer(&f, &n, json!({"accepted":true})).await;
    Box::pin(access::renew_one(&f.state, lease.clone()))
        .await
        .unwrap();
    let request = requests.recv().await.unwrap();
    assert_eq!(request.operation, Operation::AuthorityRenew);
    assert_eq!(
        request.authority.as_ref().unwrap().lease_id,
        authority.lease_id
    );
    let renewed = leases
        .find_one(doc! {"_id":&authority.lease_id})
        .await
        .unwrap()
        .unwrap();
    assert!(renewed.authority.expires_at_ms > lease.authority.expires_at_ms);
    f.state
        .db
        .collection::<Document>("assistant_conversations")
        .update_one(
            doc! {"_id": &f.chat.conversation_id},
            doc! {"$set":{"active_turn.stop_requested":true}},
        )
        .await
        .unwrap();
    assert!(
        Box::pin(access::renew_one(&f.state, lease.clone()))
            .await
            .is_err()
    );
    assert!(requests.try_recv().is_err());
    f.state
        .db
        .collection::<Document>("assistant_conversations")
        .update_one(
            doc! {"_id": &f.chat.conversation_id},
            doc! {"$set":{"active_turn.stop_requested":false}},
        )
        .await
        .unwrap();
    Box::pin(access::configure(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &n.id,
        selection(2, Capabilities::default()),
    ))
    .await
    .unwrap();
    assert!(Box::pin(access::renew_one(&f.state, lease)).await.is_err());
    assert!(requests.try_recv().is_err());
    task.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_flag_off_new_nyxbot_machines_snapshot_at_use_with_live_acl() {
    let f = orchestrator_fixture("machine_access_flag_off_new").await;
    access::ensure_indexes(&f.state.db).await.unwrap();
    let before = Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
        .await
        .unwrap();
    let n = v2(&f).await;
    assert!(!before.assignments.contains_key(&n.id));
    let rows = Box::pin(access::visible_assignments(&f.state.db, &f.chat))
        .await
        .unwrap();
    let (_, assignment) = rows.iter().find(|(node, _)| node.id == n.id).unwrap();
    assert!(assignment.legacy);
    assert_eq!(
        assignment.capabilities,
        Capabilities::legacy(n.machine.as_ref().unwrap())
    );
    assert!(
        Box::pin(access::authorize(
            &f.state.db,
            &f.chat,
            &n,
            Operation::Exec,
            &json!({})
        ))
        .await
        .is_ok()
    );
    // First direct use also works without requiring a discovery call.
    let direct = node(&f, &f.owner).await;
    assert!(
        Box::pin(access::admit(
            &f.state.db,
            &f.chat,
            &direct,
            Operation::Exec,
            &json!({}),
            None
        ))
        .await
        .unwrap()
        .is_none()
    );
    let outsider = node(&f, &uuid::Uuid::new_v4().to_string()).await;
    assert!(
        Box::pin(access::authorize(
            &f.state.db,
            &f.chat,
            &outsider,
            Operation::Exec,
            &json!({})
        ))
        .await
        .is_err()
    );
    let snapshot = Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
        .await
        .unwrap();
    assert!(!snapshot.assignments.contains_key(&outsider.id));
    enable(&f).await;
    assert_eq!(
        Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
            .await
            .unwrap(),
        snapshot
    );
    let later = v2(&f).await;
    assert!(
        Box::pin(access::authorize(
            &f.state.db,
            &f.chat,
            &later,
            Operation::Exec,
            &json!({})
        ))
        .await
        .is_err()
    );
    // Turning on the editor retains the earlier legacy snapshot.
    assert!(
        Box::pin(access::authorize(
            &f.state.db,
            &f.chat,
            &n,
            Operation::Exec,
            &json!({})
        ))
        .await
        .is_ok()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_flag_off_specialist_grants_snapshot_legacy() {
    let f = fixture("machine_access_flag_off_grant").await;
    access::ensure_indexes(&f.state.db).await.unwrap();
    Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
        .await
        .unwrap();
    let n = v2(&f).await;
    Box::pin(super::assistant_team_service::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        super::assistant_team_service::GrantChange::Machine {
            base: Box::new(super::assistant_team_service::GrantChange::Add(
                Default::default(),
            )),
            machines: Some(vec![n.id.clone()]),
            logins: None,
            mode: super::assistant_team_service::MachineGrantMode::Add,
        },
    ))
    .await
    .unwrap();
    let p = Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
        .await
        .unwrap();
    assert!(p.assignments[&n.id].legacy);
    assert_eq!(
        p.assignments[&n.id].capabilities,
        Capabilities::legacy(n.machine.as_ref().unwrap())
    );
    assert!(
        Box::pin(access::authorize(
            &f.state.db,
            &f.chat,
            &n,
            Operation::Exec,
            &json!({})
        ))
        .await
        .is_ok()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_flag_off_new_specialist_creation_snapshots_legacy() {
    let f = orchestrator_fixture("machine_access_flag_off_create").await;
    access::ensure_indexes(&f.state.db).await.unwrap();
    let n = v2(&f).await;
    let (agent, _) = Box::pin(super::assistant_team_service::create_specialist(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        super::assistant_team_service::CreateRequest {
            machines: Some(vec![n.id.clone()]),
            logins: None,
            name: "legacy-worker".into(),
            description: "Work on the selected machine".into(),
            display_name: None,
            persona: None,
            targets: Default::default(),
            account_read: false,
            specialty: None,
            created_by: "user",
        },
    ))
    .await
    .unwrap()
    .unwrap();
    let p = Box::pin(access::policy(&f.state.db, &agent.id))
        .await
        .unwrap();
    assert!(p.assignments[&n.id].legacy);
    assert_eq!(
        p.assignments[&n.id].capabilities,
        Capabilities::legacy(n.machine.as_ref().unwrap())
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_admission_is_per_node_counts_only_live_v2_and_keeps_cancellation() {
    let f = orchestrator_fixture("machine_access_admission_capacity").await;
    access::ensure_indexes(&f.state.db).await.unwrap();
    let n = v2(&f).await;
    let first = Box::pin(access::admit(
        &f.state.db,
        &f.chat,
        &n,
        Operation::Exec,
        &json!({}),
        None,
    ))
    .await
    .unwrap()
    .unwrap();
    assert!(first.expires_at_ms > chrono::Utc::now().timestamp_millis() + 30_000);
    let leases = f.state.db.collection::<model::Lease>(model::LEASES);
    let template = leases
        .find_one(doc! {"_id":&first.lease_id})
        .await
        .unwrap()
        .unwrap();
    let mut rows = Vec::new();
    for _ in 1..access::MAX_NODE_LEASES {
        let mut row = template.clone();
        row.id = uuid::Uuid::new_v4().to_string();
        row.authority.lease_id = row.id.clone();
        rows.push(row);
    }
    leases.insert_many(rows).await.unwrap();
    let error = Box::pin(access::admit(
        &f.state.db,
        &f.chat,
        &n,
        Operation::Exec,
        &json!({}),
        None,
    ))
    .await
    .unwrap_err();
    assert!(matches!(error, AppError::MachineAuthorityBusy));
    assert_eq!(error.error_code(), 12422);
    use axum::response::IntoResponse;
    assert_eq!(
        error.into_response().status(),
        axum::http::StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        leases
            .count_documents(doc! {"node_id":&n.id})
            .await
            .unwrap(),
        access::MAX_NODE_LEASES
    );
    let job = super::machine_service::issue_job(&f.state.db, &f.chat, &n, 120, vec![])
        .await
        .unwrap();
    assert!(
        Box::pin(access::admit(
            &f.state.db,
            &f.chat,
            &n,
            Operation::JobCancel,
            &json!({"job_id":job.id}),
            None
        ))
        .await
        .unwrap()
        .is_some()
    );
    let other = v2(&f).await;
    assert!(
        Box::pin(access::admit(
            &f.state.db,
            &f.chat,
            &other,
            Operation::Exec,
            &json!({}),
            None
        ))
        .await
        .unwrap()
        .is_some()
    );
    // The old global 500-row bound is gone. Other-node and expired rows do not count.
    let mut rows = Vec::new();
    for _ in 0..501 {
        let mut row = template.clone();
        row.id = uuid::Uuid::new_v4().to_string();
        row.authority.lease_id = row.id.clone();
        row.node_id = other.id.clone();
        rows.push(row);
    }
    leases.insert_many(rows).await.unwrap();
    let old = node(&f, &f.owner).await;
    assert!(
        Box::pin(access::admit(
            &f.state.db,
            &f.chat,
            &old,
            Operation::Exec,
            &json!({}),
            None
        ))
        .await
        .unwrap()
        .is_none()
    );
    assert_eq!(
        leases
            .count_documents(doc! {"node_id":&old.id})
            .await
            .unwrap(),
        0
    );
    f.state
        .db
        .collection::<Document>(model::LEASES)
        .update_many(
            doc! {"node_id":&n.id},
            doc! {"$set":{"authority.expires_at_ms":0_i64}},
        )
        .await
        .unwrap();
    assert!(
        Box::pin(access::admit(
            &f.state.db,
            &f.chat,
            &n,
            Operation::Exec,
            &json!({}),
            None
        ))
        .await
        .unwrap()
        .is_some()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_separated_requires_support_flag_card_and_keeps_mode_with_old_writer() {
    let f = orchestrator_fixture("machine_access_separated").await;
    enable(&f).await;
    let mut n = v2(&f).await;
    let caps = Capabilities {
        browser: true,
        files: true,
        ..Default::default()
    };
    let mut selected = selection(1, caps);
    selected.mode = Some("separated".into());
    selected.saved_login_ids = Some(vec![]);
    assert!(
        Box::pin(access::configure(
            &f.state.db,
            &f.owner,
            &f.chat.agent_id,
            &n.id,
            selected.clone()
        ))
        .await
        .is_err()
    );
    super::feature_flag_service::set_platform_override(
        &f.state.db,
        access::CONTEXT_FLAG,
        &super::feature_flag_service::FlagTarget::Global,
        true,
        &f.owner,
    )
    .await
    .unwrap();
    assert!(
        Box::pin(access::configure(
            &f.state.db,
            &f.owner,
            &f.chat.agent_id,
            &n.id,
            selected.clone()
        ))
        .await
        .is_err(),
        "v2 alone is insufficient"
    );
    n.machine.as_mut().unwrap().separation_evidence = Some(supported_evidence());
    n.machine.as_mut().unwrap().separated = Some(nyxid_machine::context::Support {
        available: true,
        landlock_abi: Some(6),
        reason: None,
    });
    f.state
        .db
        .collection::<Document>(crate::models::node::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&n.id},
            doc! {"$set":{"machine":bson::to_bson(&n.machine).unwrap()}},
        )
        .await
        .unwrap();
    let args = json!({"machine":n.id,"selection":selected});
    let (card, pending) = Box::pin(crate::handlers::machine_access::native(
        &f.state, &f.chat, &args,
    ))
    .await
    .unwrap();
    assert!(pending);
    assert!(
        Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
            .await
            .unwrap()
            .assignments
            .is_empty()
    );
    let id = card["acknowledgement_id"].as_str().unwrap();
    super::assistant_acknowledgement_service::decide(
        &f.state.db,
        &f.owner,
        &f.chat.conversation_id,
        id,
        true,
    )
    .await
    .unwrap();
    let mut approved = args;
    approved["acknowledgement_id"] = id.into();
    assert!(
        !Box::pin(crate::handlers::machine_access::native(
            &f.state, &f.chat, &approved
        ))
        .await
        .unwrap()
        .1
    );
    let a = Box::pin(access::admit(
        &f.state.db,
        &f.chat,
        &n,
        Operation::Browser,
        &json!({}),
        None,
    ))
    .await
    .unwrap()
    .unwrap();
    assert_eq!(a.mode, "separated");
    assert!(a.require_v2);
    assert_eq!(a.generation, 1);
    super::feature_flag_service::set_platform_override(
        &f.state.db,
        access::CONTEXT_FLAG,
        &super::feature_flag_service::FlagTarget::Global,
        false,
        &f.owner,
    )
    .await
    .unwrap();
    let mut old_writer = selection(2, caps);
    old_writer.saved_login_ids = Some(vec![]);
    let policy = Box::pin(access::configure(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &n.id,
        old_writer,
    ))
    .await
    .unwrap();
    assert_eq!(policy.assignments[&n.id].mode, "separated");
    let effective = Box::pin(access::assignment(&f.state.db, &f.chat, &n))
        .await
        .unwrap();
    assert_eq!(effective.mode, "separated");
    let visible = Box::pin(access::visible_assignments(&f.state.db, &f.chat))
        .await
        .unwrap();
    assert!(
        visible
            .iter()
            .any(|(node, assignment)| node.id == n.id && assignment.mode == "separated")
    );
    // Existing Grants removal/re-addition must retain the separated preference,
    // with all capabilities off, even when its creation flag has been disabled.
    let mut agent = f
        .state
        .db
        .collection::<AssistantAgent>(AGENTS)
        .find_one(doc! {"_id":&f.chat.agent_id})
        .await
        .unwrap()
        .unwrap();
    let before = agent.machine_node_ids.clone();
    agent.machine_node_ids.clear();
    let mut session = f.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    Box::pin(access::membership_changed(
        &f.state.db,
        &f.owner,
        &mut agent,
        &before,
        false,
        &mut session,
    ))
    .await
    .unwrap();
    agent.machine_node_ids.push(n.id.clone());
    Box::pin(access::membership_changed(
        &f.state.db,
        &f.owner,
        &mut agent,
        &[],
        false,
        &mut session,
    ))
    .await
    .unwrap();
    let retained = &agent.machine_access.as_ref().unwrap().assignments[&n.id];
    assert_eq!(retained.mode, "separated");
    assert_eq!(retained.capabilities, Capabilities::default());
    assert!(!retained.legacy);
    session.abort_transaction().await.unwrap();
    n.machine.as_mut().unwrap().separated = None;
    assert!(matches!(
        Box::pin(access::assignment(&f.state.db, &f.chat, &n)).await,
        Err(AppError::MachineAuthorityUnsupported)
    ));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_graphical_separated_action_does_not_require_capability_editor() {
    let f = orchestrator_fixture("machine_access_graphical_context").await;
    let mut n = v2(&f).await;
    n.machine.as_mut().unwrap().separation_evidence = Some(supported_evidence());
    n.machine.as_mut().unwrap().separated = Some(nyxid_machine::context::Support {
        available: true,
        landlock_abi: Some(6),
        reason: None,
    });
    f.state
        .db
        .collection::<Document>(crate::models::node::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &n.id},
            doc! {"$set": {"machine": bson::to_bson(&n.machine).unwrap()}},
        )
        .await
        .unwrap();
    super::feature_flag_service::set_platform_override(
        &f.state.db,
        access::CONTEXT_FLAG,
        &super::feature_flag_service::FlagTarget::Global,
        true,
        &f.owner,
    )
    .await
    .unwrap();
    let mut selected = selection(
        1,
        Capabilities {
            browser: true,
            files: true,
            ..Default::default()
        },
    );
    selected.mode = Some("separated".into());
    selected.saved_login_ids = Some(Vec::new());
    let policy = Box::pin(access::configure(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &n.id,
        selected,
    ))
    .await
    .unwrap();
    assert_eq!(policy.assignments[&n.id].mode, "separated");
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_separated_login_binding_and_delete_quarantine_are_atomic() {
    let f = orchestrator_fixture("machine_access_context_login").await;
    enable(&f).await;
    let mut n = v2(&f).await;
    n.machine.as_mut().unwrap().separation_evidence = Some(supported_evidence());
    n.machine.as_mut().unwrap().separated = Some(nyxid_machine::context::Support {
        available: true,
        landlock_abi: Some(6),
        reason: None,
    });
    f.state
        .db
        .collection::<Document>(crate::models::node::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&n.id},
            doc! {"$set":{"machine":bson::to_bson(&n.machine).unwrap()}},
        )
        .await
        .unwrap();
    super::feature_flag_service::set_platform_override(
        &f.state.db,
        access::CONTEXT_FLAG,
        &super::feature_flag_service::FlagTarget::Global,
        true,
        &f.owner,
    )
    .await
    .unwrap();
    let login = super::saved_login_service::put(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.owner,
        None,
        super::saved_login_service::Input {
            label: "Fixture".into(),
            allowed_origins: vec!["https://example.test".into()],
            username: zeroize::Zeroizing::new("private-fixture".into()),
            password: None,
            totp_secret: None,
            confirm_each_sign_in: false,
        },
    )
    .await
    .unwrap();
    let mut s = selection(
        1,
        Capabilities {
            browser: true,
            ..Default::default()
        },
    );
    s.mode = Some("separated".into());
    s.saved_login_ids = Some(vec![login.id.clone()]);
    Box::pin(access::configure(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &n.id,
        s,
    ))
    .await
    .unwrap();
    let a = Box::pin(access::admit(
        &f.state.db,
        &f.chat,
        &n,
        Operation::FillLogin,
        &json!({"login":login.id}),
        None,
    ))
    .await
    .unwrap()
    .unwrap();
    let row = f
        .state
        .db
        .collection::<Document>(model::CONTEXTS)
        .find_one(doc! {"_id":&a.context_id,"login_ids":&login.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!format!("{row:?}").contains("private-fixture"));
    Box::pin(super::saved_login_service::delete(
        &f.state.db,
        &f.owner,
        &login.id,
    ))
    .await
    .unwrap();
    let context = f
        .state
        .db
        .collection::<model::Context>(model::CONTEXTS)
        .find_one(doc! {"_id":&a.context_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(context.generation, 2);
    assert!(f.state.db.collection::<Document>(model::OUTBOX).find_one(doc!{"agent_id":&f.chat.agent_id,"quarantine_profiles":true,"revision":{"$gt":a.revision}}).await.unwrap().is_some());
    let next = Box::pin(access::admit(
        &f.state.db,
        &f.chat,
        &n,
        Operation::Browser,
        &json!({}),
        None,
    ))
    .await
    .unwrap()
    .unwrap();
    assert_eq!(next.generation, 2);
    assert!(next.revision > a.revision);
    assert!(
        Box::pin(access::admit(
            &f.state.db,
            &f.chat,
            &n,
            Operation::FillLogin,
            &json!({"login":login.id}),
            None
        ))
        .await
        .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn specialist_machine_login_request_is_denied_audited_and_routed_when_allowed() {
    use super::{
        assistant_acknowledgement_service as acks, assistant_settings_service as settings,
    };
    use crate::models::assistant_settings::SpecialistAccessRequests;
    let f = fixture("strict_machine_login_permission").await;
    for kind in ["machine", "saved_login"] {
        let target = uuid::Uuid::new_v4().to_string();
        settings::update(
            &f.state.db,
            &f.owner,
            settings::Update {
                specialist_access_requests: Some(SpecialistAccessRequests::Denied),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let error = crate::handlers::machine_tools::permission(
            &f.state,
            &f.chat,
            kind,
            &target,
            "Requested resource",
        )
        .await
        .expect_err("strict permission request refused");
        let denied = super::assistant_account_tools::error_result(error);
        assert!(denied.is_error);
        assert_eq!(denied.value, acks::access_request_denied());
        let audit = f
            .state
            .db
            .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .find_one(
                doc! {"event_type":"assistant_access_request_denied", "event_data.target": &target},
            )
            .await
            .unwrap()
            .expect("resource-specific denial audit");
        let data = audit.get_document("event_data").unwrap();
        assert_eq!(data.get_str("requester_agent").unwrap(), f.chat.agent_id);
        assert_eq!(data.get_str("thread").unwrap(), f.row.id);
        assert_eq!(data.get_str("key").unwrap(), f.row.credential_api_key_id);
        assert_eq!(data.get_str("target").unwrap(), target);
        assert_eq!(data.get_str("scope").unwrap(), kind);
        assert_eq!(data.get_str("decision").unwrap(), "denied");
        let cards = f
            .state
            .db
            .collection::<Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME);
        assert_eq!(
            cards
                .count_documents(doc! {"conversation_id": &f.row.id,"service_id": &target})
                .await
                .unwrap(),
            0
        );
        settings::update(
            &f.state.db,
            &f.owner,
            settings::Update {
                specialist_access_requests: Some(SpecialistAccessRequests::RoutedToNyxbot),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let routed = crate::handlers::machine_tools::permission(
            &f.state,
            &f.chat,
            kind,
            &target,
            "Requested resource",
        )
        .await
        .unwrap();
        assert_eq!(routed["error"], "acknowledgement_required");
        // Machine and saved-login grants are permission requests: NyxBot decides.
        assert_eq!(routed["decider"], "orchestrator");
        let id = routed["acknowledgement_id"].as_str().unwrap();
        assert!(
            cards
                .find_one(doc! {"_id":id,"conversation_id":&f.row.id})
                .await
                .unwrap()
                .is_some()
        );
        let queued = f.state.db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .find_one(doc! {"_id": &f.nyxbot_thread, "pending_events": {"$elemMatch": {"text": {"$regex": id}}}})
            .await.unwrap();
        assert!(queued.is_some(), "NyxBot pending event");
    }
}

#[tokio::test]
async fn specialist_machine_capability_card_is_denied_audited_and_routed_when_allowed() {
    use crate::models::assistant_settings::SpecialistAccessRequests;
    use crate::services::assistant_settings_service as settings;
    let f = fixture("strict_machine_capability_card").await;
    enable(&f).await;
    let n = v2(&f).await;
    f.state
        .db
        .collection::<Document>(crate::models::assistant_settings::COLLECTION_NAME)
        .delete_one(doc! {"_id":&f.owner})
        .await
        .unwrap();
    let revision = Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
        .await
        .unwrap()
        .revision;
    let args = json!({"agent":f.chat.agent_id,"machine":n.id,"selection":selection(revision,Capabilities{files:true,..Default::default()})});
    let (denied, pending) = Box::pin(crate::handlers::machine_access::native(
        &f.state, &f.chat, &args,
    ))
    .await
    .unwrap();
    assert!(pending);
    assert_eq!(denied["error"], "access_request_denied");
    assert_eq!(
        f.state
            .db
            .collection::<Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
            .count_documents(doc! {"conversation_id":&f.row.id})
            .await
            .unwrap(),
        0
    );
    let audit = f
        .state
        .db
        .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! {"event_type":"assistant_access_request_denied"})
        .await
        .unwrap()
        .expect("capability denial audit");
    let data = audit.get_document("event_data").unwrap();
    assert_eq!(data.get_str("target").unwrap(), n.id);
    assert_eq!(data.get_str("scope").unwrap(), "machine_capabilities");
    assert_eq!(data.get_str("decision").unwrap(), "denied");
    settings::update(
        &f.state.db,
        &f.owner,
        settings::Update {
            specialist_access_requests: Some(SpecialistAccessRequests::RoutedToNyxbot),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (routed, pending) = Box::pin(crate::handlers::machine_access::native(
        &f.state, &f.chat, &args,
    ))
    .await
    .unwrap();
    assert!(pending);
    // Auto preserves the owner action card.
    assert_eq!(routed["kind"], "action");
    assert_eq!(routed["decider"], "user");
    assert!(routed["acknowledgement_id"].is_string());
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
    let (routed, pending) = Box::pin(crate::handlers::machine_access::native(
        &f.state, &f.chat, &args,
    ))
    .await
    .unwrap();
    assert!(pending);
    assert_eq!(routed["kind"], "machine");
    assert_eq!(routed["decider"], "orchestrator");
    assert!(routed.get("confirm_phrase").is_none());
    let rows = super::assistant_acknowledgement_service::history(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    assert!(
        !rows
            .iter()
            .any(|r| r.status == "pending" && r.decider == "user")
    );
    assert!(
        rows.iter()
            .any(|r| r.status == "pending" && r.decider == "orchestrator")
    );
    assert!(
        !access::policy(&f.state.db, &f.chat.agent_id)
            .await
            .unwrap()
            .assignments
            .get(&n.id)
            .is_some_and(|a| a.capabilities.files)
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn full_access_machine_capabilities_widens_with_receipt_while_auto_asks() {
    use super::{
        assistant_acknowledgement_service as acks, assistant_settings_service as settings,
    };
    use crate::models::assistant_settings::AccessMode;
    for (mode, asks) in [(AccessMode::Autopilot, false), (AccessMode::Nyxmode, true)] {
        let f = orchestrator_fixture("mode_machine_capabilities").await;
        enable(&f).await;
        let n = v2(&f).await;
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
        let revision = access::policy(&f.state.db, &f.chat.agent_id)
            .await
            .unwrap()
            .revision;
        access::configure(
            &f.state.db,
            &f.owner,
            &f.chat.agent_id,
            &n.id,
            selection(revision, Capabilities::default()),
        )
        .await
        .unwrap();
        let before = access::policy(&f.state.db, &f.chat.agent_id).await.unwrap();
        let caps = Capabilities {
            files: true,
            ..Default::default()
        };
        let args = json!({"agent": f.chat.agent_id, "machine": n.id, "selection": selection(before.revision, caps)});
        let (value, error) = crate::handlers::machine_access::native(&f.state, &f.chat, &args)
            .await
            .unwrap();
        let current = access::policy(&f.state.db, &f.chat.agent_id).await.unwrap();
        let rows = acks::history(&f.state.db, &f.owner, &f.row.id)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        if asks {
            assert!(error);
            assert_eq!(value["error"], "acknowledgement_required");
            assert_eq!(current.revision, before.revision);
            assert!(!current.assignments[&n.id].capabilities.files);
            assert_eq!(rows[0].status, "pending");
        } else {
            assert!(!error, "{value}");
            assert!(current.assignments[&n.id].capabilities.files);
            assert_eq!(rows[0].status, "allowed");
            assert_eq!(rows[0].decided_by.as_deref(), Some("access_mode"));
            assert_eq!(rows[0].confirmation.as_ref().unwrap().reason, "access_mode");
            assert_eq!(
                rows[0].tool_name.as_deref(),
                Some("nyxid__machine_capabilities")
            );
            assert!(rows[0].summary.contains(&n.name));
        }
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn full_access_machine_pairing_completes_with_receipt_while_auto_asks() {
    use super::{
        assistant_acknowledgement_service as acks, assistant_settings_service as settings,
        machine_setup_service as setup,
    };
    use crate::models::assistant_settings::AccessMode;
    for (mode, asks) in [(AccessMode::Autopilot, false), (AccessMode::Nyxmode, true)] {
        let f = orchestrator_fixture("mode_machine_pairing").await;
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
        let pairing = setup::initiate(
            &f.state.db,
            f.state.auth_device_hmac_key.as_slice(),
            "owner-vm",
            "linux",
            "127.0.0.1",
            vec!["files".into()],
        )
        .await
        .unwrap();
        let args = json!({"code": pairing.code});
        let (result, error) = crate::handlers::machine_setup::pair_tool(&f.state, &f.chat, &args)
            .await
            .unwrap();
        let machine = setup::by_code(
            &f.state.db,
            f.state.auth_device_hmac_key.as_slice(),
            &pairing.code,
        )
        .await
        .unwrap();
        let rows = acks::history(&f.state.db, &f.owner, &f.row.id)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        if asks {
            assert!(
                !error,
                "pairing retains its existing refusal-result transport"
            );
            assert_eq!(result["error"], "acknowledgement_required");
            assert_eq!(machine.status, "pending");
            assert_eq!(rows[0].status, "pending");
        } else {
            assert!(!error, "{result}");
            assert_eq!(result["status"], "approved");
            assert_eq!(machine.status, "approved");
            assert_eq!(machine.user_id, f.owner);
            assert_eq!(rows[0].status, "allowed");
            assert_eq!(rows[0].decided_by.as_deref(), Some("access_mode"));
            assert_eq!(rows[0].confirmation.as_ref().unwrap().reason, "access_mode");
            assert!(rows[0].summary.contains("owner-vm"));
            assert!(result.get("confirm_phrase").is_none());
        }
        f.state.db.drop().await.unwrap();
    }
}

/// Issue #1883: a specialist the owner talks to directly (no NyxBot
/// assignment, no shared group) gets a neutral `missing_access` answer for a
/// capability change: no card, no NyxBot event, whatever the owner's routing
/// setting, and the audit row records it as missing access.
#[tokio::test]
async fn direct_specialist_machine_capability_request_gets_missing_access_without_a_card() {
    use crate::models::assistant_settings::SpecialistAccessRequests;
    use crate::services::assistant_settings_service as settings;
    let f = super::assistant_authority_tests::direct_fixture("direct_machine_capability").await;
    assert!(!f.chat.involved());
    enable(&f).await;
    let n = v2(&f).await;
    let revision = Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
        .await
        .unwrap()
        .revision;
    let args = json!({"agent":f.chat.agent_id,"machine":n.id,"selection":selection(revision,Capabilities{files:true,..Default::default()})});
    for policy in [
        SpecialistAccessRequests::Denied,
        SpecialistAccessRequests::RoutedToNyxbot,
    ] {
        settings::update(
            &f.state.db,
            &f.owner,
            settings::Update {
                specialist_access_requests: Some(policy),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (value, pending) = Box::pin(crate::handlers::machine_access::native(
            &f.state, &f.chat, &args,
        ))
        .await
        .unwrap();
        assert!(pending);
        assert_eq!(value["error"], "missing_access", "{value}");
        assert_eq!(value["scope"], "machine_capabilities");
        assert_eq!(value["target"], n.name);
        assert_eq!(value["request_created"], false);
        assert!(
            value["grants_path"]
                .as_str()
                .unwrap()
                .contains(&f.chat.agent_id)
        );
        assert!(
            !value.to_string().to_lowercase().contains("nyxbot"),
            "{value}"
        );
    }
    assert_eq!(
        f.state
            .db
            .collection::<Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let nyxbot = super::assistant_nyxagent::get(&f.state.db, &f.owner, &f.nyxbot_thread)
        .await
        .unwrap();
    assert!(nyxbot.pending_events.is_empty());
    let audit = f
        .state
        .db
        .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! {"event_type":"assistant_access_request_denied",
        "event_data.scope":"machine_capabilities"})
        .await
        .unwrap()
        .expect("missing-access audit");
    let data = audit.get_document("event_data").unwrap();
    assert_eq!(data.get_str("target").unwrap(), n.id);
    assert_eq!(data.get_str("decision").unwrap(), "missing_access");
    assert!(!data.get_bool("involved").unwrap());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn issue_1844_legacy_deadlines_progress_in_bounded_batches_without_partial_bound() {
    let f = fixture("issue1844_batched_observation").await;
    enable(&f).await;
    let n = v2(&f).await;
    let outbox = f.state.db.collection::<Document>(model::OUTBOX);
    outbox.insert_many((0..257).map(|_| doc! {"_id":uuid::Uuid::new_v4().to_string(),"agent_id":&f.chat.agent_id,"node_id":&n.id,"pending":true,"revision":2_i64,"claim_until":bson::DateTime::from_millis(0)})).await.unwrap();
    for expected in [128, 256, 257] {
        let dto = serde_json::to_value(
            crate::handlers::machine_access::options(&f.state, &f.owner, &f.chat.agent_id)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            outbox
                .count_documents(
                    doc! {"agent_id":&f.chat.agent_id,"execution_expires_at":{"$exists":true}}
                )
                .await
                .unwrap(),
            expected
        );
        assert_eq!(dto[0]["revocation_initialization_pending"], expected < 257);
        assert_eq!(dto[0]["revocation_expires_at"].is_null(), expected < 257);
        assert!(dto[0]["revocation_diagnostic"].is_null());
        assert_eq!(dto[0]["revocation_pending"], true);
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn issue_1844_deadline_initialization_excludes_unobserved_commits() {
    let f = fixture("issue1844_observation").await;
    let outbox = f.state.db.collection::<Document>(model::OUTBOX);
    let observed = uuid::Uuid::new_v4().to_string();
    outbox
        .insert_one(doc! {"_id":&observed,"pending":true})
        .await
        .unwrap();
    let row = outbox
        .find_one(doc! {"_id":&observed})
        .await
        .unwrap()
        .unwrap();
    let unseen = uuid::Uuid::new_v4().to_string();
    outbox
        .insert_one(doc! {"_id":&unseen,"pending":true})
        .await
        .unwrap();
    let after_commit = f
        .state
        .db
        .run_command(doc! {"hello":1})
        .await
        .unwrap()
        .get_datetime("localTime")
        .unwrap()
        .timestamp_millis();
    access::initialize_observed_revocations(&f.state.db, &[row.get_str("_id").unwrap().to_owned()])
        .await
        .unwrap();
    let initialized = outbox
        .find_one(doc! {"_id":&observed})
        .await
        .unwrap()
        .unwrap();
    assert!(
        initialized
            .get_datetime("expiry_observed_at")
            .unwrap()
            .timestamp_millis()
            >= after_commit
    );
    assert!(
        initialized
            .get_datetime("execution_expires_at")
            .unwrap()
            .timestamp_millis()
            >= after_commit + 47_000
    );
    let new_row = outbox
        .find_one(doc! {"_id":&unseen})
        .await
        .unwrap()
        .unwrap();
    assert!(!new_row.contains_key("execution_expires_at"));
    assert!(!new_row.contains_key("expiry_observed_at"));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn issue_1844_revocation_uses_one_database_clock_across_replicas() {
    use futures::TryStreamExt;
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};
    let commands = Arc::new(Mutex::new(Vec::<Document>::new()));
    let recorded = commands.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "issue1844_database_clock",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                recorded.lock().unwrap().push(event.command);
            }
        }),
    )
    .await;
    let outbox = db.collection::<Document>(model::OUTBOX);
    let id = uuid::Uuid::new_v4().to_string();
    let agent = uuid::Uuid::new_v4().to_string();
    let node = uuid::Uuid::new_v4().to_string();
    // Legacy creation timestamps cannot supply a clock or restart the grace.
    outbox
        .insert_one(doc! {"_id":&id,"agent_id":&agent,"node_id":&node,
        "pending":true,"created_at":bson::DateTime::from_millis(0)})
        .await
        .unwrap();
    let before = db.run_command(doc! {"hello":1}).await.unwrap();
    commands.lock().unwrap().clear();
    let status = access::revocation_status(&db, &agent, std::slice::from_ref(&node))
        .await
        .unwrap();
    assert!(status[&node].get_bool("execution_pending").unwrap());
    let issued = outbox.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
    let after = db.run_command(doc! {"hello":1}).await.unwrap();
    let observed = issued.get_datetime("expiry_observed_at").unwrap();
    assert!(observed >= before.get_datetime("localTime").unwrap());
    assert!(observed <= after.get_datetime("localTime").unwrap());
    assert_eq!(
        issued
            .get_datetime("execution_expires_at")
            .unwrap()
            .timestamp_millis()
            - observed.timestamp_millis(),
        47_000
    );
    // Check the actual commands sent by production, rather than a copied test
    // pipeline: both sides must reference the database clock, even when the
    // application replicas' clocks would disagree. Literal app dates regress it.
    {
        let sent = commands.lock().unwrap();
        let update = sent
            .iter()
            .find(|command| command.get_str("update").ok() == Some(model::OUTBOX))
            .unwrap();
        let set = update.get_array("updates").unwrap()[0]
            .as_document()
            .unwrap()
            .get_array("u")
            .expect("deadline initialization must be a database expression")[0]
            .as_document()
            .unwrap()
            .get_document("$set")
            .unwrap();
        assert_eq!(set.get_str("expiry_observed_at").unwrap(), "$$NOW");
        assert_eq!(
            set.get_document("execution_expires_at")
                .unwrap()
                .get_array("$add")
                .unwrap(),
            &vec![
                bson::Bson::String("$$NOW".into()),
                bson::Bson::Int64(47_000)
            ]
        );
        let aggregate = sent
            .iter()
            .find(|command| command.get_str("aggregate").ok() == Some(model::OUTBOX))
            .unwrap();
        let group = aggregate.get_array("pipeline").unwrap()[1]
            .as_document()
            .unwrap()
            .get_document("$group")
            .unwrap();
        let comparison = group
            .get_document("execution_pending")
            .unwrap()
            .get_document("$max")
            .unwrap()
            .get_array("$cond")
            .unwrap()[1]
            .as_document()
            .unwrap()
            .get_array("$gt")
            .unwrap();
        assert_eq!(comparison[1].as_str(), Some("$$NOW"));
    }
    // Exercise the real projection on each side of the DB deadline without
    // sleeping or changing any host clock. Delivery and quarantine stay durable.
    for (offset, pending) in [(5_000_i64, true), (-1, false)] {
        outbox
            .update_one(
                doc! {"_id":&id},
                vec![doc! {"$set":{
                "execution_expires_at":{"$add":["$$NOW",offset]},"quarantine_profiles":true}}],
            )
            .await
            .unwrap();
        let status = access::revocation_status(&db, &agent, std::slice::from_ref(&node))
            .await
            .unwrap();
        assert_eq!(
            status[&node].get_bool("execution_pending").unwrap(),
            pending
        );
        assert!(status[&node].get_bool("quarantine_pending").unwrap());
        let rows: Vec<Document> = outbox
            .find(doc! {"_id":&id,"pending":true})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].contains_key("acknowledged_at"));
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn issue_1844_revocation_expiry_preserves_delivery_quarantine_and_legacy_recovery() {
    let f = fixture("issue1844_pending").await;
    enable(&f).await;
    let n = v2(&f).await;
    let outbox = f.state.db.collection::<Document>(model::OUTBOX);
    let id = uuid::Uuid::new_v4().to_string();
    outbox.insert_one(doc! {"_id":&id,"agent_id":&f.chat.agent_id,"node_id":&n.id,"revision":2_i64,"pending":true,"claim_until":bson::DateTime::from_millis(0)}).await.unwrap();
    let read = || async {
        serde_json::to_value(
            crate::handlers::machine_access::options(&f.state, &f.owner, &f.chat.agent_id)
                .await
                .unwrap(),
        )
        .unwrap()
    };
    let first = read().await;
    assert_eq!(first[0]["revocation_pending"], true);
    let initialized = outbox.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
    let deadline = initialized.get_datetime("execution_expires_at").unwrap();
    assert!(deadline.timestamp_millis() >= chrono::Utc::now().timestamp_millis() + 45_000);
    let _ = read().await;
    assert_eq!(
        outbox
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap()
            .get_datetime("execution_expires_at")
            .unwrap(),
        deadline,
        "reading never restarts recovery grace"
    );
    outbox
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"execution_expires_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    let expired = read().await;
    assert_eq!(expired[0]["revocation_pending"], false);
    assert_eq!(expired[0]["delivery_pending"], true);
    let row = outbox.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
    assert!(row.get_bool("pending").unwrap());
    assert!(!row.contains_key("acknowledged_at"));
    // Expiry cannot perform profile quarantine or claim its acknowledgement.
    outbox
        .update_one(doc! {"_id":&id}, doc! {"$set":{"quarantine_profiles":true}})
        .await
        .unwrap();
    let quarantine = read().await;
    assert_eq!(quarantine[0]["execution_revocation_pending"], false);
    assert_eq!(quarantine[0]["quarantine_pending"], true);
    assert_eq!(quarantine[0]["revocation_pending"], true);
    for malformed in [bson::Bson::Null, bson::Bson::String("yesterday".into())] {
        outbox
            .update_one(
                doc! {"_id":&id},
                doc! {"$set":{"execution_expires_at":malformed,"quarantine_profiles":false}},
            )
            .await
            .unwrap();
        let invalid = read().await;
        assert_eq!(invalid[0]["revocation_pending"], true);
        assert!(invalid[0]["revocation_expires_at"].is_null());
        assert_eq!(
            invalid[0]["revocation_diagnostic"],
            "revocation_deadline_invalid"
        );
    }
    outbox
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"execution_expires_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(crate::models::node::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&n.id},
            doc! {"$set":{"machine.authority_timing_v1":false}},
        )
        .await
        .unwrap();
    let old_node = read().await;
    assert_eq!(old_node[0]["revocation_pending"], true);
    assert_eq!(
        old_node[0]["revocation_diagnostic"],
        "authority_timing_upgrade_required"
    );
    // Deliver the retained work after expiry; only this actual ACK clears it.
    let (peer, mut requests) =
        super::machine_integration_tests::peer(&f, &n, json!({"accepted":true})).await;
    access::revoke_one(&f.state, row).await.unwrap();
    assert_eq!(
        requests.recv().await.unwrap().operation,
        Operation::AuthorityRevoke
    );
    let delivered = read().await;
    assert_eq!(delivered[0]["delivery_pending"], false);
    assert!(
        outbox
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap()
            .contains_key("acknowledged_at")
    );
    peer.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn issue_1844_revocation_projection_groups_before_limit() {
    let f = fixture("issue1844_projection").await;
    enable(&f).await;
    let a = v2(&f).await;
    let b = v2(&f).await;
    let mut rows = Vec::new();
    for i in 0..502 {
        rows.push(doc!{"_id":uuid::Uuid::new_v4().to_string(),"agent_id":&f.chat.agent_id,"node_id":if i==501 {&b.id} else {&a.id},"revision":i+2_i64,"pending":true,"execution_expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::seconds(47)),"claim_until":bson::DateTime::from_millis(0)});
    }
    f.state
        .db
        .collection::<Document>(model::OUTBOX)
        .insert_many(rows)
        .await
        .unwrap();
    let dto = serde_json::to_value(
        crate::handlers::machine_access::options(&f.state, &f.owner, &f.chat.agent_id)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(dto.as_array().unwrap().len(), 2);
    assert!(
        dto.as_array()
            .unwrap()
            .iter()
            .all(|row| row["revocation_pending"] == true)
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn issue_1844_runtime_restart_refreshes_admission_and_old_renewals_stay_dead() {
    let f = fixture("issue1844_runtime").await;
    enable(&f).await;
    let old_node = v2(&f).await;
    access::configure(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &old_node.id,
        selection(
            1,
            Capabilities {
                shell: true,
                files: true,
                ..Default::default()
            },
        ),
    )
    .await
    .unwrap();
    let first = access::admit(
        &f.state.db,
        &f.chat,
        &old_node,
        Operation::Exec,
        &json!({}),
        None,
    )
    .await
    .unwrap()
    .unwrap();
    let lease = f
        .state
        .db
        .collection::<model::Lease>(model::LEASES)
        .find_one(doc! {"_id":&first.lease_id})
        .await
        .unwrap()
        .unwrap();
    let mut node = old_node.clone();
    node.machine.as_mut().unwrap().runtime_id = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<Document>(crate::models::node::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&node.id},
            doc! {"$set":{"machine":bson::to_bson(&node.machine).unwrap()}},
        )
        .await
        .unwrap();
    assert!(matches!(
        access::admit(
            &f.state.db,
            &f.chat,
            &old_node,
            Operation::Exec,
            &json!({}),
            None
        )
        .await,
        Err(AppError::MachineAuthorityStale)
    ));
    assert!(access::renew_one(&f.state, lease).await.is_err());
    let fresh = access::admit(
        &f.state.db,
        &f.chat,
        &node,
        Operation::Exec,
        &json!({}),
        None,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(fresh.runtime_id, node.machine.unwrap().runtime_id);
    assert_ne!(fresh.lease_id, first.lease_id);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn issue_1844_renewal_snapshot_cannot_cross_grant_mutation() {
    let f = fixture("issue1844_renewal_race").await;
    enable(&f).await;
    let n = v2(&f).await;
    access::configure(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &n.id,
        selection(
            1,
            Capabilities {
                shell: true,
                ..Default::default()
            },
        ),
    )
    .await
    .unwrap();
    let a = access::admit(&f.state.db, &f.chat, &n, Operation::Exec, &json!({}), None)
        .await
        .unwrap()
        .unwrap();
    let lease = f
        .state
        .db
        .collection::<model::Lease>(model::LEASES)
        .find_one(doc! {"_id":&a.lease_id})
        .await
        .unwrap()
        .unwrap();
    let mut session = f.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    // A real Mongo snapshot sees the old grants, then the revocation commits.
    f.state
        .db
        .collection::<Document>(AGENTS)
        .find_one(doc! {"_id":&f.chat.agent_id})
        .session(&mut session)
        .await
        .unwrap()
        .unwrap();
    access::configure(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &n.id,
        selection(2, Capabilities::default()),
    )
    .await
    .unwrap();
    assert!(
        access::fence_renewal_in_session(&f.state.db, &lease, &mut session)
            .await
            .is_err()
    );
    let _ = session.abort_transaction().await;
    let mut session = f.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        access::fence_renewal_in_session(&f.state.db, &lease, &mut session)
            .await
            .is_err(),
        "a fresh transaction cannot extend the old revision either"
    );
    let _ = session.abort_transaction().await;
    f.state.db.drop().await.unwrap();
}

// Task-local and database-scoped: pause only an explicitly selected renewal's
// first real Mongo snapshot. No synthetic errors, timing sleeps or node effects.
#[derive(Clone)]
struct RenewalSnapshotHook {
    database: String,
    collision: super::api_key_mutation_service::TransactionCollisionHook,
    resume: std::sync::Arc<tokio::sync::Semaphore>,
}
tokio::task_local! {
    static RENEWAL_SNAPSHOT_HOOK: RenewalSnapshotHook;
}
pub(super) async fn renewal_snapshot_loaded(
    db: &mongodb::Database,
    lease: &model::Lease,
    session: &mut mongodb::ClientSession,
) -> crate::errors::AppResult<()> {
    let Some(hook) = RENEWAL_SNAPSHOT_HOOK.try_with(Clone::clone).ok() else {
        return Ok(());
    };
    if hook.database != db.name() {
        return Ok(());
    }
    hook.collision.begin_attempt();
    db.collection::<Document>(AGENTS)
        .find_one(doc! {"_id":&lease.authority.agent_id})
        .session(session)
        .await?;
    if hook.collision.attempts() == 1 {
        hook.collision.after_reads().await;
        hook.resume.acquire().await.unwrap().forget();
    }
    Ok(())
}

#[tokio::test]
async fn issue_1844_concurrent_renewals_retry_live_fences_without_replaying_control() {
    use super::api_key_mutation_service::TransactionCollisionHook;
    use std::sync::Arc;
    let f = Box::pin(fixture("issue1844_renewal_contention")).await;
    enable(&f).await;
    let n = v2(&f).await;
    Box::pin(access::configure(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &n.id,
        selection(
            1,
            Capabilities {
                shell: true,
                ..Default::default()
            },
        ),
    ))
    .await
    .unwrap();
    let leases = f.state.db.collection::<model::Lease>(model::LEASES);
    let mut originals = Vec::new();
    for _ in 0..8 {
        let authority = Box::pin(access::admit(
            &f.state.db,
            &f.chat,
            &n,
            Operation::Exec,
            &json!({}),
            None,
        ))
        .await
        .unwrap()
        .unwrap();
        originals.push(
            leases
                .find_one(doc! {"_id":&authority.lease_id})
                .await
                .unwrap()
                .unwrap(),
        );
    }
    let (peer, mut requests) =
        super::machine_integration_tests::peer(&f, &n, json!({"accepted":true})).await;
    let barrier = Arc::new(tokio::sync::Barrier::new(originals.len() + 1));
    let resume = Arc::new(tokio::sync::Semaphore::new(0));
    let hooks = originals
        .iter()
        .map(|_| RenewalSnapshotHook {
            database: f.state.db.name().to_owned(),
            collision: TransactionCollisionHook::new(barrier.clone()),
            resume: resume.clone(),
        })
        .collect::<Vec<_>>();
    let state = &f.state;
    let renewals = futures::future::join_all(originals.iter().zip(&hooks).map(|(lease, hook)| {
        RENEWAL_SNAPSHOT_HOOK.scope(hook.clone(), async move {
            // Match the production sweep's complete caller budget, including
            // transient transaction retries and the single signed control send.
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                Box::pin(access::renew_one(state, lease.clone())),
            )
            .await
            .expect("valid concurrent renewal must fit the sweep budget")
            .expect("same-node/agent conflicts must retry")
        })
    }));
    let release = async {
        barrier.wait().await;
        resume.add_permits(originals.len());
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(renewals, release)
    })
    .await
    .unwrap();
    assert!(
        hooks
            .iter()
            .map(|hook| hook.collision.attempts())
            .sum::<usize>()
            > originals.len(),
        "shared snapshots must cause a real Mongo write conflict and retry"
    );
    let mut seen = std::collections::BTreeSet::new();
    for _ in &originals {
        let request = requests.recv().await.unwrap();
        assert_eq!(request.operation, Operation::AuthorityRenew);
        let authority = request.authority.unwrap();
        assert!(
            seen.insert(authority.lease_id.clone()),
            "one control send per renewal"
        );
        let stored = f
            .state
            .db
            .collection::<Document>(model::LEASES)
            .find_one(doc! {"_id":&authority.lease_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.get_i64("renewal_fence").unwrap(), 1);
        assert_eq!(
            stored
                .get_document("authority")
                .unwrap()
                .get_i64("expires_at_ms")
                .unwrap(),
            authority.expires_at_ms
        );
    }
    assert!(requests.try_recv().is_err());

    // A competing grant mutation after the first snapshot causes the same
    // transient conflict, but the retried live revision check must refuse it.
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let hook = RenewalSnapshotHook {
        database: f.state.db.name().to_owned(),
        collision: TransactionCollisionHook::new(barrier.clone()),
        resume: Arc::new(tokio::sync::Semaphore::new(0)),
    };
    let renewal = RENEWAL_SNAPSHOT_HOOK.scope(
        hook.clone(),
        Box::pin(access::renew_one(&f.state, originals[0].clone())),
    );
    let revoke = async {
        barrier.wait().await;
        Box::pin(access::configure(
            &f.state.db,
            &f.owner,
            &f.chat.agent_id,
            &n.id,
            selection(2, Capabilities::default()),
        ))
        .await
        .unwrap();
        hook.resume.add_permits(1);
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(renewal, revoke)
    })
    .await
    .unwrap();
    assert!(matches!(result, Err(AppError::MachinePermissionRevoked)));
    assert!(
        hook.collision.attempts() >= 2,
        "stale snapshot retries against the new revision"
    );
    assert!(
        requests.try_recv().is_err(),
        "revoked authority sends no renewal control"
    );
    peer.abort();
    f.state.db.drop().await.unwrap();
}

fn supported_evidence() -> nyxid_machine::context::Evidence {
    nyxid_machine::context::Evidence {
        version: nyxid_machine::context::EVIDENCE_VERSION,
        os: "linux".into(),
        landlock_abi: Some(6),
        separate_users: Some(true),
        credential_custody: Some(true),
        managed_browser_configured: Some(true),
        command_seccomp: Some(true),
        landlock_enforced: Some(true),
        managed_browser_sandbox: Some(true),
        managed_browser_policy: Some(true),
        ..Default::default()
    }
}

#[tokio::test]
async fn machine_access_existing_separated_requires_current_custody_and_stays_visible() {
    let f = orchestrator_fixture("machine_existing_custody").await;
    enable(&f).await;
    let mut machine = v2(&f).await;
    let mut policy = model::Policy::default();
    policy.assignments.insert(
        machine.id.clone(),
        model::Assignment {
            mode: "separated".into(),
            capabilities: Capabilities {
                shell: true,
                files: true,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    f.state.db.collection::<Document>(AGENTS).update_one(
        doc! {"_id":&f.chat.agent_id},
        doc! {"$set":{"machine_access":bson::to_bson(&policy).unwrap(),"machine_node_ids":[&machine.id]}},
    ).await.unwrap();
    for (evidence, reason) in [
        (
            Some(nyxid_machine::context::Evidence {
                version: 3,
                ..supported_evidence()
            }),
            Some("separated_probe_version_unsupported"),
        ),
        (
            Some(nyxid_machine::context::Evidence {
                credential_custody: None,
                ..supported_evidence()
            }),
            Some("node_custody_unverified"),
        ),
        (
            Some(nyxid_machine::context::Evidence {
                credential_custody: Some(false),
                ..supported_evidence()
            }),
            Some("node_custody_unverified"),
        ),
        (None, Some("separated_capabilities_not_reported")),
        (Some(supported_evidence()), None),
    ] {
        let profile = machine.machine.as_mut().unwrap();
        profile.separation_evidence = evidence;
        profile.separated = Some(nyxid_machine::context::Support {
            available: true,
            landlock_abi: Some(6),
            reason: None,
        });
        assert_eq!(access::supports_separated(profile), reason.is_none());
        f.state
            .db
            .collection::<Document>(crate::models::node::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&machine.id},
                doc! {"$set":{"machine":bson::to_bson(&machine.machine).unwrap()}},
            )
            .await
            .unwrap();
        let admitted = access::admit(
            &f.state.db,
            &f.chat,
            &machine,
            Operation::Exec,
            &json!({}),
            None,
        )
        .await;
        if reason.is_some() {
            let error = admitted.unwrap_err();
            assert!(matches!(error, AppError::MachineAuthorityUnsupported));
            assert_eq!(error.error_code(), 12419);
            for collection in [model::CONTEXTS, model::LEASES] {
                assert_eq!(
                    f.state
                        .db
                        .collection::<Document>(collection)
                        .count_documents(doc! {"node_id":&machine.id})
                        .await
                        .unwrap(),
                    0,
                    "refusal must precede context and lease effects"
                );
            }
        } else {
            assert_eq!(admitted.unwrap().unwrap().mode, "separated");
        }
        let visible = access::visible_assignments(&f.state.db, &f.chat)
            .await
            .unwrap();
        let (projected, assignment) = visible.iter().find(|(n, _)| n.id == machine.id).unwrap();
        assert_eq!(assignment, &policy.assignments[&machine.id]);
        let support = projected
            .machine
            .as_ref()
            .unwrap()
            .separated
            .as_ref()
            .unwrap();
        assert_eq!(support.available, reason.is_none());
        assert_eq!(support.reason.as_deref(), reason);
        let definitions = access::definitions(&f.state.db, &f.chat).await.unwrap();
        assert_eq!(
            definitions
                .iter()
                .any(|tool| super::machine_tools::operation(&tool.name)
                    .is_some_and(|op| op != Operation::JobCancel)),
            reason.is_none()
        );
        assert!(
            definitions
                .iter()
                .any(|tool| super::machine_tools::operation(&tool.name)
                    == Some(Operation::JobCancel)),
            "cancellation remains available even when new execution is refused"
        );
        let list =
            crate::handlers::machine_tools::call(&f.state, &f.chat, "nyx__machine_list", json!({}))
                .await
                .unwrap();
        let row = list["machines"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == machine.id)
            .unwrap();
        assert_eq!(row["access"]["mode"], "separated");
        if let Some(reason) = reason {
            assert_eq!(row["status"], "unavailable");
            assert_eq!(row["unavailable_reason"], reason);
        } else {
            assert_eq!(
                row["status"],
                serde_json::to_value(&machine.status).unwrap()
            );
        }
        let options = serde_json::to_value(
            crate::handlers::machine_access::options(&f.state, &f.owner, &f.chat.agent_id)
                .await
                .unwrap(),
        )
        .unwrap();
        let option = options
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["node_id"] == machine.id)
            .unwrap();
        let note = super::machine_tools::execution_note("separated", reason.is_none());
        assert_eq!(row["execution_note"], note);
        assert_eq!(option["execution_note"], note);
        assert_eq!(option["separated"]["available"], reason.is_none());
        assert_eq!(option["separated"]["reason"], json!(reason));
        assert_eq!(option["separated_setup_note"].is_null(), reason.is_none());
        assert_eq!(
            access::policy(&f.state.db, &f.chat.agent_id).await.unwrap(),
            policy
        );
        let stored: crate::models::node::Node = f
            .state
            .db
            .collection(crate::models::node::COLLECTION_NAME)
            .find_one(doc! {"_id":&machine.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.machine, machine.machine,
            "derived support must never overwrite the node report"
        );
    }
    // The same missing custody still permits an explicitly shared assignment.
    let shared = v2(&f).await;
    let mut shared_assignment = policy.assignments[&machine.id].clone();
    shared_assignment.mode = "shared_legacy".into();
    policy
        .assignments
        .insert(shared.id.clone(), shared_assignment);
    f.state
        .db
        .collection::<Document>(AGENTS)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"machine_access":bson::to_bson(&policy).unwrap(),"machine_node_ids":[&machine.id,&shared.id]}},
        )
        .await
        .unwrap();
    assert_eq!(
        access::admit(
            &f.state.db,
            &f.chat,
            &shared,
            Operation::Exec,
            &json!({}),
            None
        )
        .await
        .unwrap()
        .unwrap()
        .mode,
        "shared_legacy"
    );
    f.state.db.drop().await.unwrap();
}

#[test]
fn machine_access_current_support_requires_advertisement_and_authority() {
    let mut profile = nyxid_machine::MachineProfile {
        authority_versions: vec![2],
        separation_evidence: Some(supported_evidence()),
        separated: Some(nyxid_machine::context::Support {
            available: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(access::supports_separated(&profile));
    profile.separated.as_mut().unwrap().available = false;
    assert_eq!(
        access::separated_support(&profile).reason.as_deref(),
        Some("separated_context_unavailable")
    );
    profile.separated = None;
    assert!(!access::supports_separated(&profile));
    profile.authority_versions.clear();
    assert_eq!(
        access::separated_support(&profile).reason.as_deref(),
        Some("machine_authority_upgrade_required")
    );
}

#[test]
fn machine_access_new_assignment_selection_matrix() {
    use nyxid_machine::context::{Choice, EVIDENCE_VERSION};
    for evidence in [
        None,
        Some(supported_evidence()),
        Some(nyxid_machine::context::Evidence {
            os: "macos".into(),
            ..supported_evidence()
        }),
        Some(nyxid_machine::context::Evidence {
            version: EVIDENCE_VERSION - 1,
            ..supported_evidence()
        }),
    ] {
        let supported = evidence
            .as_ref()
            .is_some_and(|e| e.unavailable_reason().is_none());
        for choice in [
            None,
            Some(Choice::Automatic),
            Some(Choice::RequireSeparated),
            Some(Choice::SharedLegacy),
        ] {
            for editing in [false, true] {
                let profile = nyxid_machine::MachineProfile {
                    shell: true,
                    files: true,
                    authority_versions: vec![2],
                    separation_evidence: evidence.clone(),
                    separation_choice: choice,
                    ..Default::default()
                };
                let assignment = access::selected_assignment(&profile, 7, editing, true);
                let expected = if choice == Some(Choice::SharedLegacy) {
                    "shared_legacy"
                } else if choice.is_some() && supported {
                    "separated"
                } else {
                    model::SELECTION_REQUIRED
                };
                assert_eq!(assignment.mode, expected, "{choice:?}/{evidence:?}");
                assert_eq!(
                    assignment.capabilities.shell,
                    expected != model::SELECTION_REQUIRED && !editing
                );
                assert_eq!(assignment.revision, 7);
                if expected == "separated" {
                    assert!(!assignment.legacy);
                    assert_eq!(assignment.saved_login_ids, Some(vec![]));
                }
                if expected == model::SELECTION_REQUIRED {
                    assert_eq!(assignment.capabilities, Capabilities::default());
                }
            }
        }
    }
    let profile = nyxid_machine::MachineProfile {
        authority_versions: vec![2],
        separation_evidence: Some(supported_evidence()),
        separation_choice: Some(Choice::Automatic),
        ..Default::default()
    };
    assert_eq!(
        access::selected_assignment(&profile, 1, false, false).mode,
        model::SELECTION_REQUIRED,
        "context rollout flag never falls back to legacy"
    );
}

#[tokio::test]
async fn machine_access_durable_cutover_preserves_old_reachability_and_blocks_new_missing_choice() {
    let f = orchestrator_fixture("machine_separation_cutover").await;
    let mut old = node(&f, &f.owner).await;
    old.machine.as_mut().unwrap().separation_choice = None;
    f.state
        .db
        .collection::<Document>(crate::models::node::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&old.id},
            doc! {"$set":{"machine":bson::to_bson(&old.machine).unwrap()}},
        )
        .await
        .unwrap();
    access::ensure_indexes(&f.state.db).await.unwrap();
    let marker = f
        .state
        .db
        .collection::<Document>("platform_settings")
        .find_one(doc! {"_id":model::SEPARATION_CUTOVER})
        .await
        .unwrap()
        .unwrap();
    // Skip the older capabilities migration to exercise W9.4 lazy compatibility.
    f.state
        .db
        .collection::<Document>(AGENTS)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"machine_access":bson::to_bson(&model::Policy::default()).unwrap()}},
        )
        .await
        .unwrap();
    assert!(
        Box::pin(access::authorize(
            &f.state.db,
            &f.chat,
            &old,
            Operation::Exec,
            &json!({})
        ))
        .await
        .is_ok()
    );
    let mut new = node(&f, &f.owner).await;
    new.machine.as_mut().unwrap().separation_choice = None;
    // Strictly after the durable boundary, independent of clock resolution.
    let after =
        marker.get_datetime("created_at").unwrap().to_chrono() + chrono::Duration::milliseconds(1);
    new.created_at = after;
    f.state.db.collection::<Document>(crate::models::node::COLLECTION_NAME).update_one(doc! {"_id":&new.id},doc! {"$set":{"created_at":bson::DateTime::from_chrono(after),"machine":bson::to_bson(&new.machine).unwrap()}}).await.unwrap();
    let error = Box::pin(access::authorize(
        &f.state.db,
        &f.chat,
        &new,
        Operation::Exec,
        &json!({}),
    ))
    .await
    .unwrap_err();
    assert!(
        matches!(error,AppError::ValidationError(ref message) if message.contains("machine_selection_required") && message.contains("Assistant → Machines"))
    );
    let rows = Box::pin(access::visible_assignments(&f.state.db, &f.chat))
        .await
        .unwrap();
    assert_eq!(
        rows.iter()
            .find(|(node, _)| node.id == new.id)
            .unwrap()
            .1
            .mode,
        model::SELECTION_REQUIRED
    );
    let result = crate::handlers::machine_tools::call(
        &f.state,
        &f.chat,
        "nyx__machine_exec",
        json!({"machine":new.id,"command":"true"}),
    )
    .await
    .unwrap();
    assert_eq!(result["status"], "selection_required");
    assert!(result["settings_path"].as_str().unwrap().contains(&new.id));
    let list =
        crate::handlers::machine_tools::call(&f.state, &f.chat, "nyx__machine_list", json!({}))
            .await
            .unwrap();
    assert!(
        list["machines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == new.id && row["status"] == "selection_required")
    );
    // A later startup must not move the compatibility boundary.
    access::ensure_indexes(&f.state.db).await.unwrap();
    assert_eq!(
        f.state
            .db
            .collection::<Document>("platform_settings")
            .find_one(doc! {"_id":model::SEPARATION_CUTOVER})
            .await
            .unwrap()
            .unwrap(),
        marker
    );
    // Explicit denial wins over the compatibility snapshot forever.
    f.state.db.collection::<Document>(AGENTS).update_one(doc! {"_id":&f.chat.agent_id},doc! {"$set":{format!("machine_access.assignments.{}.capabilities",old.id):bson::to_bson(&Capabilities::default()).unwrap()}}).await.unwrap();
    assert!(matches!(
        Box::pin(access::authorize(
            &f.state.db,
            &f.chat,
            &old,
            Operation::Exec,
            &json!({})
        ))
        .await,
        Err(AppError::MachinePermissionRevoked)
    ));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_cutover_does_not_infer_machine_enablement_or_org_promotion() {
    use crate::{
        models::{org_membership::OrgRole, user::UserType},
        test_utils::{test_membership, test_user},
    };
    let f = orchestrator_fixture("machine_separation_snapshot").await;
    let org = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    let membership = test_membership(&org, &f.owner, OrgRole::Viewer, None);
    f.state
        .db
        .collection::<crate::models::org_membership::OrgMembership>(
            crate::models::org_membership::COLLECTION_NAME,
        )
        .insert_one(&membership)
        .await
        .unwrap();
    let mut org_node = node(&f, &org).await;
    let mut dormant = node(&f, &f.owner).await;
    org_node.machine.as_mut().unwrap().separation_choice = None;
    dormant.machine = Some(nyxid_machine::MachineProfile {
        version: 1,
        ..Default::default()
    });
    for n in [&org_node, &dormant] {
        f.state
            .db
            .collection::<Document>(crate::models::node::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&n.id},
                doc! {"$set":{"machine":bson::to_bson(&n.machine).unwrap()}},
            )
            .await
            .unwrap();
    }
    access::ensure_indexes(&f.state.db).await.unwrap();
    f.state
        .db
        .collection::<Document>(AGENTS)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"machine_access":bson::to_bson(&model::Policy::default()).unwrap()}},
        )
        .await
        .unwrap();
    dormant.machine.as_mut().unwrap().shell = true;
    f.state
        .db
        .collection::<Document>(crate::models::node::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&dormant.id},
            doc! {"$set":{"machine":bson::to_bson(&dormant.machine).unwrap()}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
        .update_one(doc! {"_id":membership.id}, doc! {"$set":{"role":"admin"}})
        .await
        .unwrap();
    for n in [&org_node, &dormant] {
        assert!(
            matches!(Box::pin(access::authorize(&f.state.db,&f.chat,n,Operation::Exec,&json!({}))).await,
            Err(AppError::ValidationError(message)) if message.starts_with("machine_selection_required:"))
        );
    }
    // A restart cannot extend the snapshot to these now-enabled/live paths.
    access::ensure_indexes(&f.state.db).await.unwrap();
    assert!(
        f.state
            .db
            .collection::<Document>(model::SEPARATION_COMPATIBILITY)
            .find_one(doc! {"_id":&dormant.id})
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        f.state
            .db
            .collection::<Document>(model::SEPARATION_COMPATIBILITY)
            .find_one(doc! {"_id":format!("{org}:{}",f.owner)})
            .await
            .unwrap()
            .is_none()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_old_node_requires_human_legacy_card_and_full_warning() {
    use super::assistant_acknowledgement_service as acks;
    use crate::handlers::machine_access::{
        ContextRequestBody, apply_human_context_action, request_context,
    };
    let f = orchestrator_fixture("machine_selection_legacy_card").await;
    access::ensure_indexes(&f.state.db).await.unwrap();
    let mut node = node(&f, &f.owner).await;
    node.machine.as_mut().unwrap().separation_choice = None;
    f.state
        .db
        .collection::<Document>(crate::models::node::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&node.id},
            doc! {"$set":{"machine":bson::to_bson(&node.machine).unwrap()}},
        )
        .await
        .unwrap();
    let response = request_context(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        axum::extract::Path((f.chat.agent_id.clone(), node.id.clone())),
        axum::Json(ContextRequestBody {
            selection: Selection {
                mode: Some("shared_legacy".into()),
                expected_revision: 1,
                capabilities: Capabilities::default(),
                saved_login_ids: None,
            },
        }),
    )
    .await
    .unwrap()
    .0;
    let response = serde_json::to_value(response).unwrap();
    assert_eq!(response["status"], "pending");
    let id = response["acknowledgement_id"].as_str().unwrap();
    let conversation = response["conversation_id"].as_str().unwrap();
    let card = f
        .state
        .db
        .collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>(
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        card.summary
            .contains(nyxid_machine::context::SHARED_LEGACY_WARNING)
    );
    assert!(
        !Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
            .await
            .unwrap()
            .assignments
            .contains_key(&node.id)
    );
    let allowed = acks::decide(&f.state.db, &f.owner, conversation, id, true)
        .await
        .unwrap();
    apply_human_context_action(
        &f.state,
        &f.owner,
        allowed.machine_context.as_ref().unwrap(),
    )
    .await
    .unwrap();
    let policy = Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
        .await
        .unwrap();
    let assignment = &policy.assignments[&node.id];
    assert_eq!(assignment.mode, "shared_legacy");
    assert!(assignment.legacy);
    assert_eq!(
        assignment.capabilities,
        Capabilities::legacy(node.machine.as_ref().unwrap())
    );
    assert!(
        Box::pin(access::authorize(
            &f.state.db,
            &f.chat,
            &node,
            Operation::Exec,
            &json!({})
        ))
        .await
        .is_ok()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_legacy_opt_out_requires_human_card_and_preserves_limits() {
    use super::assistant_acknowledgement_service as acks;
    use crate::handlers::machine_access::{
        ContextRequestBody, apply_human_context_action, request_context,
    };
    let f = orchestrator_fixture("machine_legacy_opt_out").await;
    access::ensure_indexes(&f.state.db).await.unwrap();
    let machine = v2(&f).await;
    let caps = Capabilities {
        files: true,
        browser: true,
        ..Default::default()
    };
    let before = model::Policy {
        revision: 7,
        assignments: std::collections::BTreeMap::from([(
            machine.id.clone(),
            model::Assignment {
                mode: "separated".into(),
                capabilities: caps,
                revision: 7,
                saved_login_ids: Some(vec![]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    f.state
        .db
        .collection::<Document>(AGENTS)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"machine_access":bson::to_bson(&before).unwrap(),"machine_node_ids":[&machine.id]}},
        )
        .await
        .unwrap();
    let context_id = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<model::Context>(model::CONTEXTS)
        .insert_one(model::Context {
            id: context_id.clone(),
            node_id: machine.id.clone(),
            agent_id: f.chat.agent_id.clone(),
            owner_id: f.owner.clone(),
            actor_id: f.owner.clone(),
            group_id: None,
            generation: 3,
            mode: "separated".into(),
        })
        .await
        .unwrap();
    let forged = Selection {
        mode: Some("shared_legacy".into()),
        expected_revision: 7,
        capabilities: Capabilities::legacy(machine.machine.as_ref().unwrap()),
        saved_login_ids: Some(vec![uuid::Uuid::new_v4().to_string()]),
    };
    let (refusal, pending) = Box::pin(crate::handlers::machine_access::native(
        &f.state,
        &f.chat,
        &json!({"machine":machine.id,"selection":forged}),
    ))
    .await
    .unwrap();
    assert!(pending);
    assert_eq!(refusal["status"], "selection_required");
    let response = request_context(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        axum::extract::Path((f.chat.agent_id.clone(), machine.id.clone())),
        axum::Json(ContextRequestBody { selection: forged }),
    )
    .await
    .unwrap()
    .0;
    let response = serde_json::to_value(response).unwrap();
    let id = response["acknowledgement_id"].as_str().unwrap();
    let conversation = response["conversation_id"].as_str().unwrap();
    assert_eq!(
        Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
            .await
            .unwrap(),
        before
    );
    let card = acks::decide(&f.state.db, &f.owner, conversation, id, true)
        .await
        .unwrap();
    assert!(
        card.summary
            .contains(nyxid_machine::context::SHARED_LEGACY_WARNING)
    );
    let action = card.machine_context.as_ref().unwrap();
    assert_eq!(action.selection.capabilities, caps);
    assert_eq!(action.selection.saved_login_ids, Some(vec![]));
    apply_human_context_action(&f.state, &f.owner, action)
        .await
        .unwrap();
    let after = Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
        .await
        .unwrap();
    assert_eq!(after.assignments[&machine.id].mode, "shared_legacy");
    assert_eq!(after.assignments[&machine.id].capabilities, caps);
    assert_eq!(after.assignments[&machine.id].saved_login_ids, Some(vec![]));
    assert_eq!(after.revision, 8);
    let context = f
        .state
        .db
        .collection::<model::Context>(model::CONTEXTS)
        .find_one(doc! {"_id":context_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(context.mode, "shared_legacy");
    assert_eq!(context.generation, 4);
    assert!(f.state.db.collection::<Document>(model::OUTBOX)
        .find_one(doc! {"agent_id":&f.chat.agent_id,"node_id":&machine.id,"revision":8,"pending":true,"quarantine_profiles":true})
        .await.unwrap().is_some());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_setup_selection_respects_context_gate_and_existing_denials() {
    use super::machine_setup_service as setup;
    let f = fixture("machine_setup_selection_gate").await;
    access::ensure_indexes(&f.state.db).await.unwrap();
    let mut machine = v2(&f).await;
    let profile = machine.machine.as_mut().unwrap();
    profile.separation_choice = Some(nyxid_machine::context::Choice::Automatic);
    profile.separation_evidence = Some(supported_evidence());
    let intent = setup::create_link(
        &f.state.db,
        &f.owner,
        None,
        crate::models::machine_setup::Choices {
            separation: Some(nyxid_machine::context::Choice::Automatic),
            legacy_warning_acknowledged: false,
            automatic_updates: None,
            owner_id: None,
            name: "selection-gate".into(),
            location: "vm".into(),
            capabilities: vec!["shell".into()],
            grant_to: Some(f.chat.agent_id.clone()),
        },
        &super::nyxbot_watch_service::ReportProvenance::LEGACY,
    )
    .await
    .unwrap();
    assert_eq!(
        Box::pin(setup::setup_selection_reason(
            &f.state.db,
            &intent,
            &machine
        ))
        .await
        .unwrap(),
        Some("machine_context_setup_not_enabled")
    );
    super::security_readiness_service::ready_test_replica(&f.state.db).await;
    super::feature_flag_service::set_platform_override(
        &f.state.db,
        access::CONTEXT_FLAG,
        &super::feature_flag_service::FlagTarget::Global,
        true,
        &f.owner,
    )
    .await
    .unwrap();
    assert_eq!(
        Box::pin(setup::setup_selection_reason(
            &f.state.db,
            &intent,
            &machine
        ))
        .await
        .unwrap(),
        None
    );
    machine.machine.as_mut().unwrap().separation_choice = None;
    assert_eq!(
        Box::pin(setup::setup_selection_reason(
            &f.state.db,
            &intent,
            &machine
        ))
        .await
        .unwrap(),
        Some("machine_selection_required")
    );
    // A stored pending assignment is not replaced by the membership writer
    // when the node later reports a usable choice. Setup must keep the owner
    // action visible rather than claim the requested grant has completed.
    f.state
        .db
        .collection::<Document>(AGENTS)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{
                format!("machine_access.assignments.{}",machine.id):bson::to_bson(&access::unresolved_assignment(1)).unwrap(),
            }},
        )
        .await
        .unwrap();
    for choice in [
        nyxid_machine::context::Choice::Automatic,
        nyxid_machine::context::Choice::SharedLegacy,
    ] {
        machine.machine.as_mut().unwrap().separation_choice = Some(choice);
        assert_eq!(
            Box::pin(setup::setup_selection_reason(
                &f.state.db,
                &intent,
                &machine
            ))
            .await
            .unwrap(),
            Some("machine_selection_required")
        );
    }
    machine.machine.as_mut().unwrap().separation_choice = None;
    // A previously reviewed assignment, including an all-off denial, retains
    // its stored authority even when the old node reports no setup intent.
    let denial = model::Assignment {
        legacy: true,
        ..Default::default()
    };
    f.state
        .db
        .collection::<Document>(AGENTS)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{
                format!("machine_access.assignments.{}",machine.id):bson::to_bson(&denial).unwrap(),
            }},
        )
        .await
        .unwrap();
    assert_eq!(
        Box::pin(setup::setup_selection_reason(
            &f.state.db,
            &intent,
            &machine
        ))
        .await
        .unwrap(),
        None
    );
    assert_eq!(
        Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
            .await
            .unwrap()
            .assignments[&machine.id],
        denial
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_access_retained_denials_bound_new_assignment_history() {
    let f = fixture("machine_assignment_history_bound").await;
    access::ensure_indexes(&f.state.db).await.unwrap();
    let n = v2(&f).await;
    let mut policy = model::Policy::default();
    for _ in 0..64 {
        policy.assignments.insert(
            uuid::Uuid::new_v4().to_string(),
            model::Assignment::default(),
        );
    }
    f.state
        .db
        .collection::<Document>(AGENTS)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"machine_access":bson::to_bson(&policy).unwrap()}},
        )
        .await
        .unwrap();
    let error = Box::pin(super::assistant_team_service::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        super::assistant_team_service::GrantChange::Machine {
            base: Box::new(super::assistant_team_service::GrantChange::Add(
                Default::default(),
            )),
            machines: Some(vec![n.id]),
            logins: None,
            mode: super::assistant_team_service::MachineGrantMode::Add,
        },
    ))
    .await
    .unwrap_err();
    assert!(
        matches!(error,AppError::ValidationError(message) if message.contains("history is full"))
    );
    assert_eq!(
        Box::pin(access::policy(&f.state.db, &f.chat.agent_id))
            .await
            .unwrap(),
        policy
    );
    f.state.db.drop().await.unwrap();
}
