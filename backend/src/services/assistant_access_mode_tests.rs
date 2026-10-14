//! Access modes (Autopilot / NyxMode / Ask Every Time) through the real
//! account, team and settings call sites, and the audits that record them.
use super::{
    assistant_acknowledgement_service as acks,
    assistant_authority_tests::{Fixture, connected, orchestrator_fixture, ordinary_key},
    assistant_settings_service as settings,
};
use crate::models::{
    assistant_acknowledgement::{AssistantAcknowledgement, COLLECTION_NAME as ACKS},
    assistant_settings::AccessMode,
};
use mongodb::bson::{self, doc};
use serde_json::{Value, json};

async fn set_mode(f: &Fixture, mode: AccessMode) {
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
}

async fn cards(f: &Fixture) -> Vec<AssistantAcknowledgement> {
    use futures::TryStreamExt;
    f.state
        .db
        .collection::<AssistantAcknowledgement>(ACKS)
        .find(doc! {"conversation_id": &f.row.id})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

async fn account(
    f: &Fixture,
    tool: &str,
    args: &Value,
) -> super::assistant_account_tools::ToolResult {
    f.tools()
        .execute(&f.auth, &format!("nyxid__{tool}"), args)
        .await
}

async fn allow(f: &Fixture, refusal: &Value) -> String {
    let id = refusal["acknowledgement_id"].as_str().unwrap().to_owned();
    acks::decide(&f.state.db, &f.owner, &f.row.id, &id, true)
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn account_tools_follow_each_access_mode_with_risk_on_the_card() {
    let f = orchestrator_fixture("mode_account_matrix").await;
    let service = connected(&f.state.db, &f.owner, "matrix", "https://matrix.example").await;
    let disable = json!({"service_id": service, "enabled": false});
    let rename = |key: &str| json!({"api_key_id": key, "name": "renamed"});

    // Autopilot: no card for any class.
    set_mode(&f, AccessMode::Autopilot).await;
    let key = ordinary_key(&f).await;
    for (tool, args) in [
        ("set_service_enabled", disable.clone()),
        ("update_agent_key", rename(&key)),
        ("delete_agent_key", json!({"api_key_id": key})),
    ] {
        let result = account(&f, tool, &args).await;
        assert!(!result.is_error, "{tool} returned an error");
    }
    assert!(cards(&f).await.is_empty());

    // NyxMode: an ordinary change runs; a sensitive change asks.
    set_mode(&f, AccessMode::Nyxmode).await;
    let key = ordinary_key(&f).await;
    assert!(
        !account(
            &f,
            "set_service_enabled",
            &json!({"service_id": service, "enabled": true})
        )
        .await
        .is_error
    );
    let refusal = account(&f, "update_agent_key", &rename(&key)).await;
    assert_eq!(refusal.value["error"], "acknowledgement_required");
    let card = cards(&f).await.pop().unwrap();
    let confirmation = card.confirmation.clone().unwrap();
    assert_eq!(confirmation.risk, "sensitive");
    assert_eq!(confirmation.access_mode, Some(AccessMode::Nyxmode));
    assert_eq!(confirmation.reason, "access_mode");
    assert!(
        card.summary.starts_with("Change agent key 'ci-bot'"),
        "card summary must mention Change agent key ci-bot"
    );
    let id = allow(&f, &refusal.value).await;
    let mut approved = rename(&key);
    approved["acknowledgement_id"] = json!(id);
    let done = account(&f, "update_agent_key", &approved).await;
    assert!(
        !done.is_error,
        "approved account tool call returned an error"
    );
    assert_eq!(done.value["name"], "renamed");
    // The card is single-use.
    assert_eq!(
        account(&f, "update_agent_key", &approved).await.value["error"],
        "acknowledgement_invalid"
    );

    // Ask Every Time: every change asks, reads never do.
    set_mode(&f, AccessMode::AskEveryTime).await;
    let before = cards(&f).await.len();
    assert!(!account(&f, "list_agent_keys", &json!({})).await.is_error);
    assert_eq!(cards(&f).await.len(), before);
    let refusal = account(&f, "set_service_enabled", &disable).await;
    assert_eq!(refusal.value["error"], "acknowledgement_required");
    let card = cards(&f)
        .await
        .into_iter()
        .find(|row| row.id == refusal.value["acknowledgement_id"])
        .unwrap();
    assert_eq!(card.confirmation.unwrap().risk, "change");
    assert_eq!(card.summary, "Disable connected service 'matrix'");
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn foreign_and_assistant_keys_are_refused_before_any_card() {
    let f = orchestrator_fixture("mode_refusal_order").await;
    set_mode(&f, AccessMode::AskEveryTime).await;
    let foreign = uuid::Uuid::new_v4().to_string();
    let refused = account(&f, "delete_agent_key", &json!({"api_key_id": foreign})).await;
    assert!(refused.is_error);
    assert_eq!(refused.value["message"], "Resource not found.");
    let own_chat_key = f.chat.api_key_id.clone();
    let refused = account(
        &f,
        "update_agent_key",
        &json!({"api_key_id": own_chat_key, "name": "x"}),
    )
    .await;
    assert!(refused.is_error);
    assert!(
        refused.value["message"]
            .as_str()
            .unwrap()
            .contains("Assistant chat keys cannot be modified"),
        "refusal must name the assistant chat key rule"
    );
    assert!(cards(&f).await.is_empty());
    // Positive control: an ordinary key does get a card in this mode.
    let key = ordinary_key(&f).await;
    let card = account(&f, "delete_agent_key", &json!({"api_key_id": key})).await;
    assert_eq!(card.value["error"], "acknowledgement_required");
    assert_eq!(cards(&f).await.len(), 1);
    f.state.db.drop().await.unwrap();
}

/// A webhook automation turn under NyxMode: the webhook policy and the access
/// mode both ask, and one approved card runs the tool exactly once.
#[tokio::test]
async fn webhook_account_tool_card_satisfies_mode_and_policy_once() {
    let mut f = orchestrator_fixture("mode_webhook_once").await;
    set_mode(&f, AccessMode::Nyxmode).await;
    let mut chat = f.chat.clone();
    chat.confirmation_policy = Some(crate::models::trigger_schedule::ConfirmationPolicy::Changes);
    f.auth.assistant_chat = Some(std::sync::Arc::new(chat));
    let key = ordinary_key(&f).await;
    let args = json!({"api_key_id": key});
    let refusal = account(&f, "delete_agent_key", &args).await;
    assert_eq!(refusal.value["error"], "acknowledgement_required");
    let card = cards(&f).await.pop().unwrap();
    assert_eq!(card.confirmation.as_ref().unwrap().reason, "webhook");
    assert_eq!(
        card.confirmation.as_ref().unwrap().access_mode,
        Some(AccessMode::Nyxmode)
    );
    assert!(
        card.summary.contains("Delete agent key 'ci-bot'"),
        "card summary must mention Delete agent key ci-bot"
    );
    let id = allow(&f, &refusal.value).await;
    let mut approved = args.clone();
    approved["acknowledgement_id"] = json!(id);
    let done = account(&f, "delete_agent_key", &approved).await;
    assert!(
        !done.is_error,
        "approved account tool call returned an error"
    );
    assert!(
        super::key_service::get_api_key(&f.state.db, &f.owner, &key)
            .await
            .is_err()
    );
    assert_eq!(cards(&f).await.len(), 1);
    f.state.db.drop().await.unwrap();
}

/// Every non-read account tool resolves a card summary against real rows.
#[tokio::test]
async fn every_changing_account_tool_has_a_card_summary() {
    let f = orchestrator_fixture("mode_card_summaries").await;
    let db = &f.state.db;
    let key = ordinary_key(&f).await;
    let service = connected(db, &f.owner, "summary-service", "https://summary.example").await;
    let credential = uuid::Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    db.collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME)
        .insert_one(
            doc! {"_id": &credential, "user_id": &f.owner, "label": "Stored",
            "credential_type": "api_key", "status": "active", "created_at": now, "updated_at": now},
        )
        .await
        .unwrap();
    let binding = super::agent_binding_service::create_binding_with_scope_authorization(
        db,
        &f.owner,
        Some(&f.owner),
        &key,
        &service,
        &credential,
    )
    .await
    .unwrap();
    let bot = uuid::Uuid::new_v4().to_string();
    db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME).insert_one(doc! {
        "_id": &bot, "user_id": &f.owner, "platform": "telegram", "label": "Build bot",
        "bot_token_encrypted": bson::Binary {subtype: bson::spec::BinarySubtype::Generic, bytes: vec![1]},
        "platform_bot_id": "1", "platform_bot_username": "b", "webhook_registered": false,
        "webhook_secret_hash": "x", "status": "active", "is_active": true,
        "created_at": now, "updated_at": now,
    }).await.unwrap();
    let node = uuid::Uuid::new_v4().to_string();
    db.collection::<bson::Document>(crate::models::node::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": &node, "user_id": &f.owner, "name": "Workstation", "status": "offline",
            "auth_token_hash": "x", "is_active": true, "created_at": now, "updated_at": now,
        })
        .await
        .unwrap();
    set_mode(&f, AccessMode::Autopilot).await;
    let route = account(
        &f,
        "set_channel_route",
        &json!({"bot_id": bot, "agent_api_key_id": key, "platform_conversation_id": "c2"}),
    )
    .await;
    assert!(!route.is_error, "channel route creation returned an error");
    let route = route.value["id"].as_str().unwrap().to_owned();
    let tools = f.tools();
    let cases = [
        (
            "update_agent_key",
            json!({"api_key_id": key, "name": "n"}),
            "Change agent key 'ci-bot'",
        ),
        (
            "delete_agent_key",
            json!({"api_key_id": key}),
            "Delete agent key 'ci-bot'",
        ),
        (
            "bind_agent_key_credential",
            json!({"api_key_id": key, "user_service_id": service, "user_api_key_id": credential}),
            "Make agent key 'ci-bot'",
        ),
        (
            "unbind_agent_key_credential",
            json!({"api_key_id": key, "binding_id": binding.id}),
            "Remove credential binding",
        ),
        (
            "update_channel_bot",
            json!({"bot_id": bot, "label": "Deploy"}),
            "Update channel bot 'Build bot'",
        ),
        (
            "delete_channel_bot",
            json!({"bot_id": bot}),
            "Delete channel bot 'Build bot'",
        ),
        (
            "delete_channel_route",
            json!({"route_id": route}),
            "Delete channel route 'c2'",
        ),
        (
            "set_channel_route",
            json!({"bot_id": bot, "agent_api_key_id": key, "platform_conversation_id": "c1"}),
            "Send chat 'c1' on channel bot 'Build bot' to agent key 'ci-bot'",
        ),
        (
            "set_service_enabled",
            json!({"service_id": service, "enabled": true}),
            "Enable connected service",
        ),
        (
            "delete_service",
            json!({"service_id": service}),
            "Delete connected service",
        ),
        (
            "set_approval_mode",
            json!({"service_id": service, "approval_mode": "grant", "approval_required": true}),
            "Set approvals for 'summary-service' to grant, approval required",
        ),
        (
            "delete_node",
            json!({"node_id": node}),
            "Delete node 'Workstation'",
        ),
    ];
    let covered: Vec<&str> = cases.iter().map(|(name, _, _)| *name).collect();
    for name in super::assistant_account_tools::TOOL_NAMES {
        if !super::assistant_account_tools::read_only(name) {
            assert!(covered.contains(name), "no card summary case for {name}");
        }
    }
    for (name, args, expected) in cases {
        let summary = tools.action_summary(&f.owner, name, &args).await.unwrap();
        assert!(
            summary.starts_with(expected),
            "{name}: action summary has the wrong prefix"
        );
    }
    db.drop().await.unwrap();
}

/// A legacy row (destructive confirmations off, no access_mode) migrates to
/// Autopilot on any write instead of failing with a $set/$setOnInsert conflict.
#[tokio::test]
async fn legacy_settings_row_migrates_on_any_write() {
    let f = orchestrator_fixture("mode_legacy_migration").await;
    let collection = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_settings::COLLECTION_NAME);
    collection
        .insert_one(
            doc! {"_id": &f.owner, "skip_destructive_confirmation": true,
            "updated_at": bson::DateTime::now()},
        )
        .await
        .unwrap();
    assert_eq!(
        settings::get(&f.state.db, &f.owner)
            .await
            .unwrap()
            .access_mode,
        AccessMode::Autopilot
    );
    let saved = settings::update(
        &f.state.db,
        &f.owner,
        settings::Update {
            timezone: Some("Asia/Singapore".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(saved.access_mode, AccessMode::Autopilot);
    assert_eq!(saved.timezone.as_deref(), Some("Asia/Singapore"));
    let raw = collection
        .find_one(doc! {"_id": &f.owner})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(raw.get_str("access_mode").unwrap(), "autopilot");
    // The legacy switch is a view of the mode: back on leaves Autopilot.
    let saved = settings::update(
        &f.state.db,
        &f.owner,
        settings::Update {
            skip_destructive_confirmation: Some(false),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(saved.access_mode, AccessMode::Nyxmode);
    assert!(!saved.skip_destructive_confirmation);
    // Turning it off from Ask Every Time is Autopilot; turning it on keeps Ask Every Time.
    set_mode(&f, AccessMode::AskEveryTime).await;
    let saved = settings::update(
        &f.state.db,
        &f.owner,
        settings::Update {
            skip_destructive_confirmation: Some(false),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(saved.access_mode, AccessMode::AskEveryTime);
    f.state.db.drop().await.unwrap();
}

/// The agent's override governs its threads, and is found by agent ID even
/// when the agent belongs to an organization rather than the acting person.
#[tokio::test]
async fn agent_override_governs_and_resolves_for_organization_agents() {
    let f = orchestrator_fixture("mode_override_org").await;
    set_mode(&f, AccessMode::Autopilot).await;
    assert_eq!(
        acks::effective_access_mode(&f.state.db, &f.chat)
            .await
            .unwrap(),
        AccessMode::Autopilot
    );
    let agents = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME);
    agents
        .update_one(
            doc! {"_id": &f.chat.agent_id},
            doc! {"$set": {"access_mode_override": "ask_every_time",
            "user_id": uuid::Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
    assert_eq!(
        acks::effective_access_mode(&f.state.db, &f.chat)
            .await
            .unwrap(),
        AccessMode::AskEveryTime
    );
    let need = acks::confirmation_needed(
        &f.state.db,
        &f.chat,
        acks::ActionFacts::of(acks::OwnerActionClass::Change),
        false,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(need.mode, Some(AccessMode::AskEveryTime));
    // Reads never ask and never read the mode.
    assert!(
        acks::confirmation_needed(
            &f.state.db,
            &f.chat,
            acks::ActionFacts::of(acks::OwnerActionClass::Read),
            false,
        )
        .await
        .unwrap()
        .is_none()
    );
    f.state.db.drop().await.unwrap();
}

/// Mode changes are audited for the workspace and for agent overrides,
/// with wire values; the agent's model change is audited too.
#[tokio::test]
async fn access_mode_and_model_changes_are_audited() {
    let f = orchestrator_fixture("mode_audits").await;
    let actor = super::audit_service::AuditActor {
        user_id: f.owner.clone(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    };
    let before = settings::get(&f.state.db, &f.owner).await.unwrap();
    set_mode(&f, AccessMode::AskEveryTime).await;
    let after = settings::get(&f.state.db, &f.owner).await.unwrap();
    settings::audit_access_mode(
        &f.state.db,
        &actor,
        None,
        Some(before.access_mode),
        Some(after.access_mode),
    )
    .await;
    settings::audit(&f.state.db, &f.owner, &before, &after).await;
    let updated = super::assistant_team_service::set_agent_policy(
        &f.state.db,
        &actor,
        &f.chat.agent_id,
        super::assistant_team_service::AgentPolicy {
            model: Some("nyxagent/fast"),
            access_mode_override: Some(Some(AccessMode::Autopilot)),
        },
    )
    .await
    .unwrap();
    assert_eq!(updated.model, "nyxagent/fast");
    assert_eq!(updated.access_mode_override, Some(AccessMode::Autopilot));
    let cleared = super::assistant_team_service::set_agent_policy(
        &f.state.db,
        &actor,
        &f.chat.agent_id,
        super::assistant_team_service::AgentPolicy {
            model: None,
            access_mode_override: Some(None),
        },
    )
    .await
    .unwrap();
    assert_eq!(cleared.access_mode_override, None);
    let logs = f
        .state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME);
    let event = |filter: bson::Document| {
        let logs = logs.clone();
        async move {
            logs.find_one(filter)
                .await
                .unwrap()
                .expect("audit row")
                .get_document("event_data")
                .unwrap()
                .clone()
        }
    };
    let workspace = event(
        doc! {"event_type": "assistant_access_mode_changed", "event_data.scope": "workspace"},
    )
    .await;
    assert_eq!(workspace.get_str("old").unwrap(), "nyxmode");
    assert_eq!(workspace.get_str("new").unwrap(), "ask_every_time");
    let set = event(
        doc! {"event_type": "assistant_access_mode_changed", "event_data.scope": "agent",
        "event_data.new": "autopilot"},
    )
    .await;
    assert_eq!(set.get_str("agent_id").unwrap(), f.chat.agent_id);
    assert!(matches!(set.get("old"), Some(bson::Bson::Null)));
    let cleared = event(
        doc! {"event_type": "assistant_access_mode_changed", "event_data.scope": "agent",
        "event_data.old": "autopilot"},
    )
    .await;
    assert!(matches!(cleared.get("new"), Some(bson::Bson::Null)));
    let settings_row = event(doc! {"event_type": "assistant_settings_updated"}).await;
    assert_eq!(
        settings_row.get_str("access_mode").unwrap(),
        "ask_every_time"
    );
    assert_eq!(
        settings_row.get_str("specialist_access_requests").unwrap(),
        "denied"
    );
    let model =
        event(doc! {"event_type": "assistant_model_changed", "event_data.scope": "agent"}).await;
    assert_eq!(model.get_str("new").unwrap(), "nyxagent/fast");
    assert_eq!(model.get_str("agent_id").unwrap(), f.chat.agent_id);
    // An unchanged override writes no audit row.
    assert_eq!(
        logs.count_documents(doc! {"event_type": "assistant_access_mode_changed"})
            .await
            .unwrap(),
        3
    );
    f.state.db.drop().await.unwrap();
}

/// The sandbox holds for every kind of specialist turn: owner, event and
/// channel turns are denied under the default policy and routed to NyxBot
/// only when the owner allows it; guest turns never raise a request.
#[tokio::test]
async fn specialist_access_requests_follow_policy_for_every_turn_kind() {
    use crate::models::{
        assistant_conversation::TurnOrigin, assistant_settings::SpecialistAccessRequests,
    };
    let f = super::assistant_authority_tests::fixture("mode_turn_kind_matrix").await;
    let service = connected(&f.state.db, &f.owner, "matrix-docs", "https://docs.example").await;
    let conversations = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME);
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
        for (origin, guest) in [
            (TurnOrigin::User, false),
            (TurnOrigin::Event, false),
            (TurnOrigin::Channel, false),
            (TurnOrigin::Channel, true),
        ] {
            conversations
                .update_one(
                    doc! {"_id": &f.row.id},
                    doc! {"$set": {"guest_turn": guest,
                    "active_turn.origin": bson::to_bson(&origin).unwrap()}},
                )
                .await
                .unwrap();
            let chat = acks::for_key(&f.state.db, &f.owner, Some(&f.row.credential_api_key_id))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(chat.guest, guest);
            let (value, card) =
                acks::service_gate(&f.state.db, &chat, &service, "matrix-docs", "Docs", false)
                    .await
                    .unwrap()
                    .unwrap();
            let expected = match (guest, policy) {
                (true, _) => "owner_only",
                (false, SpecialistAccessRequests::Denied) => "access_request_denied",
                (false, SpecialistAccessRequests::RoutedToNyxbot) => "acknowledgement_required",
            };
            assert_eq!(
                value["error"], expected,
                "{origin:?} guest={guest} {policy:?}"
            );
            if expected != "acknowledgement_required" {
                assert!(card.is_none(), "{origin:?} guest={guest} {policy:?}");
            }
        }
    }
    // Denied turns raised no card at all.
    let raised = cards(&f).await;
    assert!(raised.iter().all(|row| row.decider == "orchestrator"));
    assert_eq!(
        raised.len(),
        1,
        "one routed request, reused across turn kinds"
    );
    f.state.db.drop().await.unwrap();
}
