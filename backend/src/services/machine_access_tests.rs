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
    Box::pin(super::machine_integration_tests::ungranted_node_named(
        f, owner, &name,
    ))
    .await
}
async fn enable(f: &Fixture) {
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
    let (_, token, _) = super::node_service::create_registration_token(
        &f.state.db,
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
