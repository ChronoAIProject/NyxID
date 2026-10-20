//! Access modes (Full access / Auto / Ask every time) through the real
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

    // Full access: no card for any class.
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
    assert_eq!(cards(&f).await.len(), 2);
    assert!(
        cards(&f)
            .await
            .iter()
            .all(|row| row.decided_by.as_deref() == Some("access_mode"))
    );

    // Auto: an ordinary change runs; a sensitive change asks.
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

    // Ask every time: every change asks, reads never do.
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

/// A webhook automation turn under Auto: the webhook policy and the access
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
/// Full access on any write instead of failing with a $set/$setOnInsert conflict.
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
    // The legacy switch is a view of the mode: back on leaves Full access.
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
    // Turning it off from Ask every time is Full access; turning it on keeps Ask every time.
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
    .ask()
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
        .ask()
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

#[tokio::test]
async fn full_access_account_success_records_one_non_consumable_receipt_and_no_user_decision() {
    let f = orchestrator_fixture("mode_receipt_account").await;
    set_mode(&f, AccessMode::Autopilot).await;
    let key = ordinary_key(&f).await;
    let args = json!({"api_key_id": key, "name": "receipt-key"});
    let done = account(&f, "update_agent_key", &args).await;
    assert!(
        !done.is_error,
        "update_agent_key was refused in Full access"
    );
    assert_eq!(done.value["name"], "receipt-key");
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let count = f
                .state
                .db
                .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(
                    doc! {"event_type":"assistant_action_auto_allowed", "user_id":&f.owner},
                )
                .await
                .unwrap();
            if count == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let rows = cards(&f).await;
    assert_eq!(rows.len(), 1);
    let receipt = &rows[0];
    assert_eq!(receipt.kind, "action");
    assert_eq!(receipt.status, "allowed");
    assert_eq!(receipt.decider, "user");
    assert_eq!(receipt.decided_by.as_deref(), Some("access_mode"));
    assert_eq!(
        receipt.tool_name.as_deref(),
        Some("nyxid__update_agent_key")
    );
    assert_eq!(
        receipt.arguments_digest.as_deref(),
        Some(acks::arguments_digest(&args).as_str())
    );
    assert_eq!(
        receipt.summary,
        format!(
            "Change agent key 'ci-bot' ({}): name",
            done.value["key_prefix"].as_str().unwrap()
        )
    );
    assert_eq!(receipt.created_at, receipt.decided_at.unwrap());
    assert_eq!(receipt.expires_at, receipt.created_at);
    let confirmation = receipt.confirmation.as_ref().unwrap();
    assert_eq!(confirmation.risk, "sensitive");
    assert_eq!(confirmation.access_mode, Some(AccessMode::Autopilot));
    assert_eq!(confirmation.reason, "access_mode");
    assert!(
        acks::history(&f.state.db, &f.owner, &f.row.id)
            .await
            .unwrap()
            .iter()
            .any(|r| r.id == receipt.id && r.status == "allowed")
    );
    assert!(
        acks::decided_since(
            &f.state.db,
            &f.owner,
            &f.row.id,
            receipt.created_at - chrono::Duration::seconds(1)
        )
        .await
        .unwrap()
        .is_empty()
    );
    assert!(acks::decisions_note(&rows).is_empty());
    assert!(
        !acks::consume_action(
            &f.state.db,
            &f.chat,
            &receipt.id,
            "nyxid__update_agent_key",
            &args
        )
        .await
        .unwrap()
    );
    // Even corrupted expiry cannot turn a receipt into executable authority.
    f.state.db.collection::<bson::Document>(ACKS).update_one(doc! {"_id": &receipt.id},
        doc! {"$set": {"expires_at": bson::DateTime::from_chrono(chrono::Utc::now() + chrono::Duration::minutes(10))}}).await.unwrap();
    assert!(
        !acks::consume_action(
            &f.state.db,
            &f.chat,
            &receipt.id,
            "nyxid__update_agent_key",
            &args
        )
        .await
        .unwrap()
    );
    let mut session = f.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        !acks::consume_action_in_session(
            &f.state.db,
            &f.chat,
            &receipt.id,
            "nyxid__update_agent_key",
            &args,
            &mut session
        )
        .await
        .unwrap()
    );
    session.abort_transaction().await.unwrap();
    // Positive control: Auto raises an owner card for the same sensitive action.
    set_mode(&f, AccessMode::Nyxmode).await;
    let asked = account(
        &f,
        "update_agent_key",
        &json!({"api_key_id": key, "name": "requires-card"}),
    )
    .await;
    assert_eq!(asked.value["error"], "acknowledgement_required");
    assert_eq!(
        cards(&f)
            .await
            .iter()
            .filter(|r| r.decided_by.as_deref() == Some("access_mode"))
            .count(),
        1
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn full_access_webhook_change_executes_with_receipt_while_auto_asks() {
    for (mode, asks) in [(AccessMode::Autopilot, false), (AccessMode::Nyxmode, true)] {
        let mut f = orchestrator_fixture("mode_receipt_webhook").await;
        set_mode(&f, mode).await;
        let mut chat = f.chat.clone();
        chat.confirmation_policy =
            Some(crate::models::trigger_schedule::ConfirmationPolicy::Changes);
        f.auth.assistant_chat = Some(std::sync::Arc::new(chat));
        let service = connected(
            &f.state.db,
            &f.owner,
            "webhook-change",
            "https://webhook.example",
        )
        .await;
        let result = account(
            &f,
            "set_service_enabled",
            &json!({"service_id":service,"enabled":false}),
        )
        .await;
        let rows = cards(&f).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].confirmation.as_ref().unwrap().reason, "webhook");
        let stored = super::user_service_service::get_user_service(&f.state.db, &f.owner, &service)
            .await
            .unwrap();
        if asks {
            assert_eq!(result.value["error"], "acknowledgement_required");
            assert_eq!(rows[0].status, "pending");
            assert!(stored.is_active);
        } else {
            assert!(
                !result.is_error,
                "set_service_enabled was refused in Full access"
            );
            assert_eq!(rows[0].status, "allowed");
            assert_eq!(rows[0].decided_by.as_deref(), Some("access_mode"));
            assert_eq!(rows[0].confirmation.as_ref().unwrap().risk, "change");
            assert!(!stored.is_active);
        }
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn full_access_set_agent_operations_widens_with_receipt_while_auto_asks() {
    use super::{agent_operation_scope_service as scopes, assistant_team_service as team};
    use crate::models::{
        agent_operation_scope::OperationSelection, assistant_agent::AgentGrants,
        downstream_service::ProxyOperationRule,
    };
    for (mode, asks) in [(AccessMode::Autopilot, false), (AccessMode::Nyxmode, true)] {
        let f = orchestrator_fixture("mode_operations_widen").await;
        set_mode(&f, mode).await;
        crate::test_utils::set_agent_operation_scopes_enabled(&f.state.db, &f.owner, true).await;
        let service = connected(&f.state.db, &f.owner, "mode-ops", "http://127.0.0.1:9").await;
        let (agent, _) = team::create_specialist(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            team::CreateRequest {
                machines: None,
                logins: None,
                name: "scope-worker".into(),
                description: "Use service operations".into(),
                display_name: None,
                persona: None,
                targets: Default::default(),
                account_read: false,
                specialty: None,
                created_by: "nyxbot",
            },
        )
        .await
        .unwrap()
        .unwrap();
        team::set_grants(
            &f.state.db,
            &f.owner,
            &agent.id,
            team::GrantChange::Add(AgentGrants {
                service_ids: vec![service.clone()],
                ..Default::default()
            }),
        )
        .await
        .unwrap();
        let mut selection = OperationSelection {
            expected_revision: 0,
            all_operations: false,
            endpoint_ids: vec![],
            contract_digest: None,
            inputs: Default::default(),
            rules: vec![ProxyOperationRule {
                method: "GET".into(),
                path_template: "/items/{id}".into(),
                ..Default::default()
            }],
        };
        scopes::set(
            &f.state.db,
            &f.owner,
            &agent.id,
            &service,
            &selection,
            false,
        )
        .await
        .unwrap();
        selection.expected_revision = 1;
        selection.all_operations = true;
        selection.rules.clear();
        let args = json!({"subagent": agent.id, "service_id": service, "selection": selection});
        let (value, error) = crate::handlers::assistant_team::execute_tool(
            &f.state,
            &f.chat,
            "nyxid__set_agent_operations",
            &args,
        )
        .await;
        let current = team::agent(&f.state.db, &f.owner, &agent.id).await.unwrap();
        let rows = cards(&f).await;
        assert_eq!(rows.len(), 1);
        if asks {
            assert!(error);
            assert_eq!(value["error"], "acknowledgement_required");
            assert_eq!(current.operation_scope_revisions[&service], 1);
            assert!(current.operation_scopes.contains_key(&service));
            assert_eq!(rows[0].status, "pending");
        } else {
            assert!(!error, "{value}");
            assert_eq!(current.operation_scope_revisions[&service], 2);
            assert!(!current.operation_scopes.contains_key(&service));
            assert_eq!(rows[0].status, "allowed");
            assert_eq!(rows[0].decided_by.as_deref(), Some("access_mode"));
            assert_eq!(rows[0].confirmation.as_ref().unwrap().reason, "access_mode");
            assert_eq!(
                rows[0].tool_name.as_deref(),
                Some("nyxid__set_agent_operations")
            );
            assert!(rows[0].summary.contains("all operations"));
        }
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn full_access_transition_retires_old_owner_cards_but_preserves_auto_override() {
    let f = orchestrator_fixture("mode_retires_old_cards").await;
    let key = ordinary_key(&f).await;
    let args = json!({"api_key_id": key, "name": "after-switch"});
    let asked = account(&f, "update_agent_key", &args).await;
    assert_eq!(asked.value["error"], "acknowledgement_required");
    let actor = super::audit_service::AuditActor {
        user_id: f.owner.clone(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    };
    super::assistant_team_service::set_agent_policy(
        &f.state.db,
        &actor,
        &f.chat.agent_id,
        super::assistant_team_service::AgentPolicy {
            model: None,
            access_mode_override: Some(Some(AccessMode::Nyxmode)),
        },
    )
    .await
    .unwrap();
    set_mode(&f, AccessMode::Autopilot).await;
    assert_eq!(
        cards(&f).await[0].status,
        "pending",
        "Auto override still asks"
    );
    super::assistant_team_service::set_agent_policy(
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
    assert_eq!(cards(&f).await[0].status, "expired");
    let done = account(&f, "update_agent_key", &args).await;
    assert!(
        !done.is_error,
        "update_agent_key was refused after switching to Full access"
    );
    assert_eq!(done.value["name"], "after-switch");
    let rows = cards(&f).await;
    assert!(
        !rows
            .iter()
            .any(|r| r.status == "pending" && r.decider == "user")
    );
    assert_eq!(
        rows.iter()
            .filter(|r| r.decided_by.as_deref() == Some("access_mode"))
            .count(),
        1
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn full_access_retirement_batches_pending_asks_and_resolves_each_mode_once() {
    use futures::TryStreamExt;
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};
    let f = orchestrator_fixture("mode_pending_batch_retirement").await;
    set_mode(&f, AccessMode::Autopilot).await;
    let card = acks::request(
        &f.state.db,
        &f.chat,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some("nyxid__delete_node"),
            arguments: None,
            summary: "Old owner ask",
            platform: false,
        },
    )
    .await
    .unwrap();
    let mut pending = Vec::new();
    for _ in 1..205 {
        let mut row = card.clone();
        row.id = uuid::Uuid::new_v4().to_string();
        pending.push(row);
    }
    let agents = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME);
    let mut auto_agent = agents
        .find_one(doc! {"_id": &f.chat.agent_id})
        .await
        .unwrap()
        .unwrap();
    let auto_id = uuid::Uuid::new_v4().to_string();
    auto_agent.insert("_id", &auto_id);
    auto_agent.insert("kind", "specialist");
    auto_agent.insert("name", "auto-worker");
    auto_agent.insert("access_mode_override", "nyxmode");
    agents.insert_one(auto_agent).await.unwrap();
    let mut auto_ask = card.clone();
    auto_ask.id = uuid::Uuid::new_v4().to_string();
    auto_ask.requester_agent_id = Some(auto_id);
    let mut routed = card.clone();
    routed.id = uuid::Uuid::new_v4().to_string();
    routed.decider = "orchestrator".into();
    let mut foreign = card.clone();
    foreign.id = uuid::Uuid::new_v4().to_string();
    foreign.user_id = uuid::Uuid::new_v4().to_string();
    pending.extend([auto_ask.clone(), routed.clone(), foreign.clone()]);
    f.state
        .db
        .collection::<AssistantAcknowledgement>(ACKS)
        .insert_many(pending)
        .await
        .unwrap();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let recorded = commands.clone();
    let handler = EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            recorded.lock().unwrap().push(event.command);
        }
    });
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(handler);
    let db = mongodb::Client::with_options(options)
        .unwrap()
        .database(f.state.db.name());
    acks::retire_full_access_confirmations(&db, Some(&f.owner), None)
        .await
        .unwrap();
    let queries = commands.lock().unwrap().clone();
    let agent_reads = queries
        .iter()
        .filter(|c| c.get_str("find").ok() == Some(crate::models::assistant_agent::COLLECTION_NAME))
        .count();
    let settings_reads = queries
        .iter()
        .filter(|c| {
            c.get_str("find").ok() == Some(crate::models::assistant_settings::COLLECTION_NAME)
        })
        .count();
    assert_eq!(
        agent_reads, 2,
        "one resolution per user/agent, across batches"
    );
    assert_eq!(
        settings_reads, 1,
        "Auto override needs no owner-mode lookup"
    );
    assert!(
        !queries.iter().any(|c| c.get_str("find").ok()
            == Some(crate::models::assistant_conversation::COLLECTION_NAME)),
        "no conversation enumeration"
    );
    let ack_queries: Vec<_> = queries
        .iter()
        .filter(|c| c.get_str("find").ok() == Some(ACKS))
        .collect();
    assert!(
        ack_queries.len() >= 3,
        "the pending set exceeds two batches"
    );
    for query in ack_queries {
        assert_eq!(query.get_i64("limit").unwrap(), 100);
        let filter = query.get_document("filter").unwrap();
        assert_eq!(filter.get_str("status").unwrap(), "pending");
        assert_eq!(filter.get_str("kind").unwrap(), "action");
        assert_eq!(filter.get_str("decider").unwrap(), "user");
        assert_eq!(filter.get_str("user_id").unwrap(), f.owner);
    }
    let remaining: Vec<_> = f
        .state
        .db
        .collection::<AssistantAcknowledgement>(ACKS)
        .find(doc! {"status":"pending"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(remaining.len(), 3);
    for kept in [auto_ask.id, routed.id, foreign.id] {
        assert!(remaining.iter().any(|r| r.id == kept));
    }
    // Agent writes target just that agent's pending asks and retain its Auto policy.
    acks::retire_full_access_confirmations(&db, None, Some(&f.chat.agent_id))
        .await
        .unwrap();
    assert_eq!(
        f.state
            .db
            .collection::<AssistantAcknowledgement>(ACKS)
            .count_documents(doc! {"status":"pending"})
            .await
            .unwrap(),
        3
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn full_access_startup_retires_pre_upgrade_floor_cards_without_claiming_success() {
    let f = orchestrator_fixture("mode_retires_upgrade_floor_cards").await;
    set_mode(&f, AccessMode::Autopilot).await;
    // A card persisted by the old Full-access webhook floor before this release.
    let card = acks::request(
        &f.state.db,
        &f.chat,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some("nyxid__delete_node"),
            arguments: Some(&json!({"node_id":"old"})),
            summary: "Delete old node",
            platform: false,
        },
    )
    .await
    .unwrap();
    f.state
        .db
        .collection::<bson::Document>(ACKS)
        .update_one(
            doc! {"_id": &card.id},
            doc! {"$unset": {"requester_agent_id": ""}},
        )
        .await
        .unwrap();
    let before = acks::history(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    assert_eq!(before[0].status, "pending", "reads do not retire asks");
    acks::retire_full_access_confirmations(&f.state.db, None, None)
        .await
        .unwrap();
    acks::retire_full_access_confirmations(&f.state.db, None, None)
        .await
        .unwrap();
    let history = acks::history(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, card.id);
    assert_eq!(history[0].status, "expired");
    assert!(history[0].decided_by.is_none());
    assert!(
        acks::decided_since(
            &f.state.db,
            &f.owner,
            &f.row.id,
            chrono::Utc::now() - chrono::Duration::hours(1)
        )
        .await
        .unwrap()
        .is_empty()
    );
    f.state.db.drop().await.unwrap();
}
