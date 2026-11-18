use super::*;
use crate::services::{assistant_nyxagent as engine, feature_flag_service as flags};

pub(super) async fn activate(f: &Fixture) {
    flags::set_platform_override(
        &f.state.db,
        flags::ORG_INSTRUCTION_FLAG_KEY,
        &flags::FlagTarget::User(f.owner.clone()),
        true,
        &f.owner,
    )
    .await
    .unwrap();
}

async fn row(f: &Fixture, id: &str) -> UserService {
    crate::services::user_service_service::find_user_service_by_id(&f.state.db, id)
        .await
        .unwrap()
        .unwrap()
}

async fn selected_input(
    f: &Fixture,
    id: &str,
    source: Option<engine::OwnerInstructionSource>,
    group: bool,
    channel: bool,
) {
    f.state
        .db
        .collection::<Document>(COLLECTION)
        .delete_many(doc! {"kind":"selection"})
        .await
        .unwrap();
    let request: engine::TurnRequest =
        serde_json::from_value(json!({"text":format!("/service {id}\nRead my account")})).unwrap();
    let mut start = engine::TurnStart::from(&request);
    start.owner_instruction_source = source;
    if channel {
        start.origin = crate::models::assistant_conversation::TurnOrigin::Channel;
    }
    let mut conversation = f.row.clone();
    if group {
        conversation.group_id = Some("group-thread".into());
    }
    let mut session = f.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    selection::bind_owner_selection(
        &f.state.db,
        &mut session,
        &conversation,
        &start,
        &start.text,
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
}

#[tokio::test]
async fn org_instruction_requires_new_exact_scoped_owner_provenance() {
    let f = orchestrator_fixture("instruction_provenance").await;
    activate(&f).await;
    let owner = org(&f).await;
    let c = catalog(&f).await;
    let a = connection(&f, &owner, "a", Some(&c)).await;
    let b = connection(&f, &owner, "b", Some(&c)).await;
    let connection = row(&f, &a).await;
    for (source, group, channel, expected) in [
        (None, false, false, false),
        (None, false, true, false),
        (
            Some(engine::OwnerInstructionSource::VerifiedPrivateChannel),
            true,
            true,
            false,
        ),
        (
            Some(engine::OwnerInstructionSource::FirstPartyOwner),
            false,
            true,
            false,
        ),
        (
            Some(engine::OwnerInstructionSource::FirstPartyOwner),
            true,
            false,
            false,
        ),
        (
            Some(engine::OwnerInstructionSource::VerifiedPrivateChannel),
            false,
            true,
            true,
        ),
        (
            Some(engine::OwnerInstructionSource::FirstPartyOwner),
            false,
            false,
            true,
        ),
    ] {
        selected_input(&f, &a, source, group, channel).await;
        assert_eq!(
            instruction::owner_instruction(&f.state.db, &f.chat, &connection)
                .await
                .unwrap(),
            expected
        );
        assert!(
            !instruction::owner_instruction(&f.state.db, &f.chat, &row(&f, &b).await)
                .await
                .unwrap()
        );
    }
    let mut other = f.chat.clone();
    other.turn_id = Some("other-turn".into());
    assert!(
        !instruction::owner_instruction(&f.state.db, &other, &connection)
            .await
            .unwrap()
    );
    for field in 0..4 {
        let mut other = f.chat.clone();
        match field {
            0 => other.user_id = "another-actor".into(),
            1 => other.conversation_id = "another-conversation".into(),
            2 => other.turn_live = false,
            _ => other.turn_stopped = true,
        }
        assert!(
            !instruction::owner_instruction(&f.state.db, &other, &connection)
                .await
                .unwrap()
        );
    }
    other = f.chat.clone();
    other.api_key_id = "other-key".into();
    assert!(
        !instruction::owner_instruction(&f.state.db, &other, &connection)
            .await
            .unwrap()
    );
    f.state
        .db
        .collection::<Document>(COLLECTION)
        .update_many(
            doc! {"kind":"selection"},
            doc! {"$set":{"selection_source":"card"}},
        )
        .await
        .unwrap();
    assert!(
        !instruction::owner_instruction(&f.state.db, &f.chat, &connection)
            .await
            .unwrap()
    );
    f.state
        .db
        .collection::<Document>(COLLECTION)
        .update_many(
            doc! {"kind":"selection"},
            doc! {"$set":{"selection_source":"owner","expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}},
        )
        .await
        .unwrap();
    assert!(
        !instruction::owner_instruction(&f.state.db, &f.chat, &connection)
            .await
            .unwrap()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_card_survives_auto_to_full_and_redeems_exactly_once() {
    let f = orchestrator_fixture("instruction_mode_switch").await;
    activate(&f).await;
    let owner = org(&f).await;
    let id = connection(&f, &owner, "github", None).await;
    mode(&f, AccessMode::Nyxmode).await;
    let operation = json!({"kind":"http","method":"GET","path":"/user/repos"});
    let refusal = gate(&f, &id, &owner, &operation, None)
        .await
        .refusal
        .unwrap();
    let card = refusal["acknowledgement_id"].as_str().unwrap().to_owned();
    mode(&f, AccessMode::Autopilot).await;
    acks::retire_full_access_confirmations(&f.state.db, None, None)
        .await
        .unwrap();
    let stored = f
        .state
        .db
        .collection::<Document>("assistant_acknowledgements")
        .find_one(doc! {"_id":&card})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.get_str("status"), Ok("pending"));
    assert_eq!(
        stored
            .get_document("confirmation")
            .unwrap()
            .get_str("reason"),
        Ok("org_connection")
    );
    allow(&f, refusal).await;
    assert!(
        gate(&f, &id, &owner, &operation, Some(&card))
            .await
            .refusal
            .is_none()
    );
    assert_eq!(
        gate(&f, &id, &owner, &operation, Some(&card))
            .await
            .refusal
            .unwrap()["error"],
        "acknowledgement_invalid"
    );
    assert!(
        gate(
            &f,
            &id,
            &owner,
            &json!({"kind":"http","method":"GET","path":"/different"}),
            None
        )
        .await
        .refusal
        .is_some()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_admission_stays_mode_sensitive_while_card_identity_ignores_mode() {
    let f = orchestrator_fixture("instruction_binding_split").await;
    activate(&f).await;
    let owner = org(&f).await;
    let id = connection(&f, &owner, "github", None).await;
    mode(&f, AccessMode::Nyxmode).await;
    let operation = json!({"kind":"http","method":"GET","path":"/user/repos"});
    let refusal = gate(&f, &id, &owner, &operation, None)
        .await
        .refusal
        .unwrap();
    let card = refusal["acknowledgement_id"].as_str().unwrap().to_owned();
    allow(&f, refusal).await;
    let admission = gate(&f, &id, &owner, &operation, Some(&card))
        .await
        .admission
        .unwrap();
    let card_identity = org_card_binding(&f.state.db, &id, &f.chat.api_key_id)
        .await
        .unwrap();
    assert_eq!(
        admission.binding,
        connection_binding(&f.state.db, &id, &f.chat.api_key_id)
            .await
            .unwrap(),
        "execution keeps the mode-sensitive binding"
    );
    assert_ne!(admission.binding, card_identity);
    // Tightening the access mode after admission fences the admitted action.
    mode(&f, AccessMode::AskEveryTime).await;
    assert!(admission.revalidate(&f.state.db, &id, None).await.is_err());
    // The organization card identity does not depend on the mode.
    assert_eq!(
        card_identity,
        org_card_binding(&f.state.db, &id, &f.chat.api_key_id)
            .await
            .unwrap()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_instruction_flag_does_not_change_connection_bindings() {
    // Model consent and other persisted grants bind connection_binding; turning
    // the rule on or off must not invalidate them.
    let f = orchestrator_fixture("instruction_binding_flag").await;
    let owner = org(&f).await;
    let id = connection(&f, &owner, "model", None).await;
    let before = connection_binding(&f.state.db, &id, &f.chat.api_key_id)
        .await
        .unwrap();
    activate(&f).await;
    assert_eq!(
        before,
        connection_binding(&f.state.db, &id, &f.chat.api_key_id)
            .await
            .unwrap()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn trusted_selection_allows_org_reads_but_not_independent_destructive_confirmation() {
    let f = orchestrator_fixture("instruction_selection_gate").await;
    activate(&f).await;
    let owner = org(&f).await;
    let id = connection(&f, &owner, "github", None).await;
    selected_input(
        &f,
        &id,
        Some(engine::OwnerInstructionSource::FirstPartyOwner),
        false,
        false,
    )
    .await;
    for setting in [
        AccessMode::Autopilot,
        AccessMode::Nyxmode,
        AccessMode::AskEveryTime,
    ] {
        mode(&f, setting).await;
        assert!(
            gate(
                &f,
                &id,
                &owner,
                &json!({"method":"GET","path":"/repos"}),
                None
            )
            .await
            .refusal
            .is_none()
        );
    }
    let decision = check(
        &f.state.db,
        &f.state.node_ws_manager,
        &f.chat,
        &id,
        &owner,
        &json!({"method":"DELETE","path":"/repos/a"}),
        acks::ActionFacts::of(acks::OwnerActionClass::Destructive),
        None,
        "Delete repo",
        false,
        None,
        false,
        None,
    )
    .await
    .unwrap();
    assert!(decision.refusal.is_some());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn only_org_github_full_access_refuses_before_any_provider_or_refresh_call_with_connect_own_hint()
 {
    let f = orchestrator_fixture("instruction_before_refresh").await;
    activate(&f).await;
    mode(&f, AccessMode::Autopilot).await;
    let owner = org(&f).await;
    let c = catalog(&f).await;
    f.state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! {"_id":&c},
            doc! {"$set":{"requires_user_credential":true,"slug":"github","name":"GitHub"}},
        )
        .await
        .unwrap();
    let provider = wiremock::MockServer::start().await;
    let id = compaction::compact_connection(&f, &owner, &c, "nwnwnw413", &provider.uri()).await;
    let service = row(&f, &id).await;
    // A refresh-capable expired OAuth seed would fail decryption if materialized.
    // The normal resolver must still reach the owner card without touching it.
    f.state.db.collection::<Document>("user_api_keys").update_one(doc!{"_id":&service.api_key_id},doc!{"$set":{
        "credential_type":"oauth2","provider_config_id":"refresh-provider","expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::minutes(5)),
        "refresh_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![1,2,3]}}}).await.unwrap();
    let mut held = crate::services::proxy_service::resolve_proxy_target_by_user_service_id(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &id,
        None,
        None,
        crate::services::proxy_service::ProxyExecutionContext::new(
            None,
            crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
        )
        .with_assistant_chat(Some(&f.chat)),
    )
    .await
    .unwrap()
    .unwrap()
    .target;
    assert!(held.credential_deferred);
    assert!(held.credential.is_empty());
    let decision = check(
        &f.state.db,
        &f.state.node_ws_manager,
        &f.chat,
        &id,
        &owner,
        &json!({"kind":"http","method":"GET","path":"/repos"}),
        acks::ActionFacts::of(acks::OwnerActionClass::Read),
        None,
        "Read repos",
        false,
        Some(&held),
        false,
        None,
    )
    .await
    .unwrap();
    let refusal = decision.refusal.unwrap();
    assert_eq!(refusal["connect_own"]["supported"], true);
    assert_eq!(refusal["connect_own"]["service_slug"], "github");
    let card = f
        .state
        .db
        .collection::<Document>("assistant_acknowledgements")
        .find_one(doc! {"_id":refusal["acknowledgement_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert!(
        card.get_document("connect_own")
            .unwrap()
            .get_bool("supported")
            .unwrap()
    );
    assert!(provider.received_requests().await.unwrap().is_empty());
    // A destination swap after exact approval fails at the materialization barrier.
    let ack = allow(&f, refusal).await;
    let mut admitted = check(
        &f.state.db,
        &f.state.node_ws_manager,
        &f.chat,
        &id,
        &owner,
        &json!({"kind":"http","method":"GET","path":"/repos"}),
        acks::ActionFacts::of(acks::OwnerActionClass::Read),
        Some(&ack),
        "Read repos",
        false,
        Some(&held),
        false,
        None,
    )
    .await
    .unwrap()
    .admission
    .unwrap();
    f.state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(
            doc! {"_id":&service.endpoint_id},
            doc! {"$set":{"url":"https://changed.example"}},
        )
        .await
        .unwrap();
    assert!(
        admitted
            .materialize_http(
                &f.state.db,
                &f.state.encryption_keys,
                &mut held,
                "GET",
                "/repos",
                &HttpProjection::default()
            )
            .await
            .is_err()
    );
    assert!(provider.received_requests().await.unwrap().is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_agent_owner_type_and_platform_exceptions_are_independent_of_shared_branch() {
    let f = orchestrator_fixture("instruction_agent_owner").await;
    activate(&f).await;
    let owner = org(&f).await;
    let id = connection(&f, &owner, "org", None).await;
    assert!(
        instruction::covered(&f.state.db, &f.chat, &row(&f, &id).await)
            .await
            .unwrap()
    );
    f.state
        .db
        .collection::<Document>("assistant_agents")
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"user_id":&owner}},
        )
        .await
        .unwrap();
    assert!(
        !instruction::covered(&f.state.db, &f.chat, &row(&f, &id).await)
            .await
            .unwrap()
    );
    f.state
        .db
        .collection::<Document>("assistant_agents")
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"user_id":&f.owner}},
        )
        .await
        .unwrap();
    for patch in [
        doc! {"source":"auto_provision"},
        doc! {"source":null,"credential_binding":"platform"},
    ] {
        f.state
            .db
            .collection::<Document>("user_services")
            .update_one(doc! {"_id":&id}, doc! {"$set":patch})
            .await
            .unwrap();
        assert!(
            !instruction::covered(&f.state.db, &f.chat, &row(&f, &id).await)
                .await
                .unwrap()
        );
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn personal_specialist_org_read_needs_instruction_and_guests_cannot_bypass_it() {
    let mut f = crate::services::assistant_authority_tests::direct_fixture(
        "instruction_personal_specialist",
    )
    .await;
    activate(&f).await;
    mode(&f, AccessMode::Autopilot).await;
    let owner = org(&f).await;
    let id = connection(&f, &owner, "specialist-org", None).await;
    crate::services::assistant_team_service::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        crate::services::assistant_team_service::GrantChange::Add(
            crate::models::assistant_agent::AgentGrants {
                service_ids: vec![id.clone()],
                ..Default::default()
            },
        ),
    )
    .await
    .unwrap();
    let operation = json!({"kind":"http","method":"GET","path":"/repos"});
    let refusal = gate(&f, &id, &owner, &operation, None)
        .await
        .refusal
        .unwrap();
    let card = f
        .state
        .db
        .collection::<Document>("assistant_acknowledgements")
        .find_one(doc! {"_id":refusal["acknowledgement_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        card.get_document("confirmation").unwrap().get_str("reason"),
        Ok("org_connection")
    );
    f.chat.guest = true;
    let guest = check(
        &f.state.db,
        &f.state.node_ws_manager,
        &f.chat,
        &id,
        &owner,
        &operation,
        acks::ActionFacts::of(acks::OwnerActionClass::Read),
        None,
        "Read repos",
        false,
        None,
        false,
        None,
    )
    .await;
    assert!(guest.is_err() || guest.unwrap().admission.is_none());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_owned_specialist_keeps_existing_org_execution_in_full_access() {
    let f = crate::services::org_agent_tests::Fixture::new("instruction_org_specialist").await;
    let (agent, home) = f.create().await;
    flags::set_platform_override(
        &f.state.db,
        flags::ORG_INSTRUCTION_FLAG_KEY,
        &flags::FlagTarget::User(f.admin.clone()),
        true,
        &f.admin,
    )
    .await
    .unwrap();
    let id = f.service(&f.org, "org-specialist-service").await;
    crate::services::assistant_team_service::set_grants(
        &f.state.db,
        &f.admin,
        &agent.id,
        crate::services::assistant_team_service::GrantChange::Add(
            crate::models::assistant_agent::AgentGrants {
                service_ids: vec![id.clone()],
                ..Default::default()
            },
        ),
    )
    .await
    .unwrap();
    let row = engine::begin_turn(
        &f.state.db,
        &f.admin,
        &serde_json::from_value::<engine::TurnRequest>(
            json!({"conversation_id":home.id,"text":"Read organization data"}),
        )
        .unwrap(),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let key = f.key(&row).await;
    let auth = crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
        .await
        .unwrap();
    let chat = acks::for_key_with_access(
        &f.state.db,
        &f.admin,
        Some(&key.id),
        auth.org_agent_access.as_ref(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        !instruction::personal_agent(&f.state.db, &chat)
            .await
            .unwrap()
    );
    assert!(chat.org_agent_access.as_ref().unwrap().shared().is_none());
    let admitted = check(
        &f.state.db,
        &f.state.node_ws_manager,
        &chat,
        &id,
        &f.org,
        &json!({"kind":"http","method":"GET","path":"/repos"}),
        acks::ActionFacts::of(acks::OwnerActionClass::Read),
        None,
        "Read organization data",
        false,
        None,
        false,
        None,
    )
    .await
    .unwrap();
    assert!(admitted.refusal.is_none());
    admitted
        .admission
        .unwrap()
        .revalidate(&f.state.db, &id, None)
        .await
        .unwrap();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn native_ssh_org_connection_requires_card_in_full() {
    let f = orchestrator_fixture("instruction_native_full").await;
    activate(&f).await;
    mode(&f, AccessMode::Autopilot).await;
    let owner = org(&f).await;
    let (_, id, _) = native::ssh_fixture(&f, &owner).await;
    let response = native::ssh(&f, &id, None).await.unwrap();
    let body = axum::body::to_bytes(response.into_body(), 16384)
        .await
        .unwrap();
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert!(
        result.to_string().contains("acknowledgement_required"),
        "{result}"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_declarations_track_instruction_for_each_exact_connection() {
    use crate::services::{
        assistant_job_service_use as jobs, machine_gateway_service::AvailableService,
    };
    let f = orchestrator_fixture("instruction_machine_declarations").await;
    activate(&f).await;
    let owner = org(&f).await;
    let a = connection(&f, &owner, "org-a", None).await;
    let b = connection(&f, &owner, "org-b", None).await;
    selected_input(
        &f,
        &a,
        Some(engine::OwnerInstructionSource::FirstPartyOwner),
        false,
        false,
    )
    .await;
    let declared = vec![
        AvailableService {
            user_service: true,
            id: a.clone(),
            slug: "org-a".into(),
            inference: None,
            git: None,
        },
        AvailableService {
            user_service: true,
            id: b.clone(),
            slug: "org-b".into(),
            inference: None,
            git: None,
        },
    ];
    let plan = jobs::prepare(
        &f.state,
        &f.chat,
        "node",
        &json!({"command":"bounded work"}),
        &declared,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(plan.org_labels.len(), 2);
    assert_eq!(plan.unresolved_org_connections, vec![b.clone()]);
    assert!(!plan.legacy_org_policy);
    mode(&f, AccessMode::Autopilot).await;
    let gate = acks::confirmation_needed_with_org(
        &f.state.db,
        &f.chat,
        acks::ActionFacts::of(acks::OwnerActionClass::Change),
        false,
        acks::OrgInstructionRequirement::Required,
    )
    .await
    .unwrap();
    assert!(gate.ask().is_some());
    let pool = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<Document>("service_pools")
        .insert_one(doc! {
            "_id":&pool,"user_id":&f.owner,"slug":"org-pool","name":"Org pool","is_active":true,
            "members":[{"user_service_id":&a},{"user_service_id":&b}],
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
        })
        .await
        .unwrap();
    let pooled = jobs::prepare(
        &f.state,
        &f.chat,
        "node",
        &json!({"command":"bounded pool work"}),
        &[AvailableService {
            user_service: false,
            id: pool,
            slug: "org-pool".into(),
            inference: None,
            git: None,
        }],
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(pooled.unresolved_org_connections, vec![b.clone()]);
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id":&b},
            doc! {"$set":{"credential_binding":"platform"}},
        )
        .await
        .unwrap();
    let excluded = jobs::prepare(
        &f.state,
        &f.chat,
        "node",
        &json!({"command":"bounded work"}),
        &declared,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(excluded.unresolved_org_connections.is_empty());
    assert!(
        excluded.legacy_org_policy,
        "excluded platform declarations keep the previous policy floor"
    );
    f.state.db.drop().await.unwrap();
}
