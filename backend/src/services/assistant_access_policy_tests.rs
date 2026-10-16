//! Issue #1883 access policy: NyxBot takes part in specialist work only when
//! it assigned that work or shares a group with it; everything else is told
//! what it is missing, with no card and no NyxBot notice.
use super::*;
use crate::{
    models::assistant_acknowledgement::{
        AssistantAcknowledgement, COLLECTION_NAME as ACKS, CardRouting, bind_continuation,
    },
    services::{
        assistant_account_tools as tools, assistant_acknowledgement_service as acks,
        assistant_authority_tests::{
            chat_for_row, connected, direct_fixture, fixture, new_orchestrator, pin_assignment,
            pin_group_with_nyxbot,
        },
        assistant_nyxagent as engine, assistant_settings_service as settings,
        assistant_team_service as team,
    },
};
use chrono::Duration;
use mongodb::bson::{Document, doc};
use serde_json::json;
use uuid::Uuid;

/// The cutover stored in the marker document, read without the process cache.
async fn stored_cutover(db: &Database) -> AppResult<Option<DateTime<Utc>>> {
    db.collection::<Document>(MIGRATIONS_COLLECTION_NAME)
        .find_one(doc! {"_id": CUTOVER_MARKER_ID})
        .await?
        .map(|row| parse_cutover(&row))
        .transpose()
}

fn denied_mode() -> settings::Update {
    settings::Update {
        specialist_access_requests: Some(
            crate::models::assistant_settings::SpecialistAccessRequests::Denied,
        ),
        ..Default::default()
    }
}

fn routed_mode() -> settings::Update {
    settings::Update {
        specialist_access_requests: Some(
            crate::models::assistant_settings::SpecialistAccessRequests::RoutedToNyxbot,
        ),
        ..Default::default()
    }
}

async fn denial_notices(db: &Database, owner: &str, thread: &str) -> usize {
    engine::messages(db, owner, thread, 100, None)
        .await
        .unwrap()
        .iter()
        .filter(|message| message.error_code.as_deref() == Some("specialist_access_denied"))
        .count()
}

async fn card_count(db: &Database, filter: Document) -> u64 {
    db.collection::<Document>(ACKS)
        .count_documents(filter)
        .await
        .unwrap()
}

fn service_request<'a>(target: &'a str, kind: &'a str, tool: Option<&'a str>) -> acks::Request<'a> {
    acks::Request {
        kind,
        service: Some((target, target, "Requested resource")),
        tool,
        arguments: None,
        summary: "Request access",
        platform: false,
    }
}

/// A minimal card row of the given routing and age, for the pure checks.
fn sample_card(
    routing: Option<CardRouting>,
    created_at: DateTime<Utc>,
) -> AssistantAcknowledgement {
    let mut row = doc! {
        "_id": Uuid::new_v4().to_string(), "conversation_id": "c", "user_id": "u",
        "api_key_id": "k", "kind": "skills", "summary": "Skill", "status": "allowed",
        "requested_turn_id": bson::Bson::Null,
        "created_at": bson::DateTime::from_chrono(created_at),
        "expires_at": bson::DateTime::from_chrono(created_at + Duration::minutes(15)),
    };
    if let Some(routing) = routing {
        row.insert("routing", bson::to_bson(&routing).unwrap());
    }
    bson::from_document(row).unwrap()
}

fn chat(role: AgentRole, guest: bool, report_to: Option<&str>, with_nyxbot: bool) -> ChatAuthority {
    ChatAuthority {
        platform_thread: false,
        platform_origin: None,
        org_agent_access: None,
        turn_id: Some("turn".into()),
        turn_stopped: false,
        turn_live: true,
        machine_node_ids: Vec::new(),
        saved_login_ids: Vec::new(),
        conversation_id: "conversation".into(),
        user_id: "owner".into(),
        api_key_id: "key".into(),
        role,
        agent_id: "agent-1".into(),
        agent_name: "worker".into(),
        guest,
        confirmation_policy: None,
        report_to: report_to.map(str::to_owned),
        with_nyxbot,
    }
}

use crate::models::assistant_conversation::AgentRole;

#[test]
fn involvement_needs_a_specialist_owner_turn_with_a_report_snapshot() {
    assert!(chat(AgentRole::Subagent, false, Some("t"), false).involved());
    assert!(chat(AgentRole::Subagent, false, None, true).involved());
    assert!(!chat(AgentRole::Subagent, false, None, false).involved());
    assert!(!chat(AgentRole::Subagent, true, Some("t"), true).involved());
    assert!(!chat(AgentRole::Orchestrator, false, Some("t"), true).involved());
    let mut stopped = chat(AgentRole::Subagent, false, Some("t"), false);
    stopped.turn_stopped = true;
    assert!(!stopped.involved());
    let mut ended = chat(AgentRole::Subagent, false, Some("t"), false);
    ended.turn_live = false;
    assert!(!ended.involved());
}

#[test]
fn card_routing_round_trips_and_binds_continuations() {
    let assigned = CardRouting::Assigned {
        report_to: "thread-1".into(),
    };
    assert_eq!(
        serde_json::to_value(&assigned).unwrap(),
        json!({"kind": "assigned", "report_to": "thread-1"})
    );
    let encoded = bson::to_bson(&assigned).unwrap();
    assert_eq!(bson::from_bson::<CardRouting>(encoded).unwrap(), assigned);
    assert_eq!(
        serde_json::from_value::<CardRouting>(json!({"kind": "not_involved"})).unwrap(),
        CardRouting::NotInvolved
    );
    let event = |routing: Option<&CardRouting>| {
        bind_continuation(routing, team::event("permission_decided", "x".into(), None))
    };
    let bound = event(Some(&assigned));
    assert!(bound.bound);
    assert_eq!(bound.report_to.as_deref(), Some("thread-1"));
    let group = event(Some(&CardRouting::GroupWithNyxbot {
        group_id: "g".into(),
    }));
    assert!(group.bound && group.report_to.is_none());
    let alone = event(Some(&CardRouting::NotInvolved));
    assert!(alone.bound && alone.report_to.is_none());
    let legacy = event(None);
    assert!(!legacy.bound && legacy.report_to.is_none());
    assert!(assigned.involved() && !CardRouting::NotInvolved.involved());
}

#[test]
fn skill_notification_eligibility_matrix() {
    let cutover = Utc::now();
    let before = cutover - Duration::hours(1);
    let after = cutover + Duration::seconds(1);
    let assigned = sample_card(
        Some(CardRouting::Assigned {
            report_to: "t".into(),
        }),
        after,
    );
    assert_eq!(
        skill_notification_eligible(&assigned, Some(cutover)).unwrap(),
        SkillEligibility::Eligible(SkillDestination::Assigned("t".into()))
    );
    let group = sample_card(
        Some(CardRouting::GroupWithNyxbot {
            group_id: "g".into(),
        }),
        after,
    );
    assert_eq!(
        skill_notification_eligible(&group, Some(cutover)).unwrap(),
        SkillEligibility::Eligible(SkillDestination::GroupNyxbotHome {
            group_id: "g".into()
        })
    );
    let alone = sample_card(Some(CardRouting::NotInvolved), before);
    assert_eq!(
        skill_notification_eligible(&alone, Some(cutover)).unwrap(),
        SkillEligibility::Ineligible("not_involved")
    );
    let legacy = sample_card(None, before);
    assert_eq!(
        skill_notification_eligible(&legacy, Some(cutover)).unwrap(),
        SkillEligibility::Eligible(SkillDestination::Legacy)
    );
    let stripped = sample_card(None, after);
    assert_eq!(
        skill_notification_eligible(&stripped, Some(cutover)).unwrap(),
        SkillEligibility::Ineligible("unbound_after_cutover")
    );
    // An unread cutover fails closed for the legacy branch only.
    assert!(skill_notification_eligible(&legacy, None).is_err());
    assert!(skill_notification_eligible(&assigned, None).is_ok());
}

#[test]
fn missing_access_copy_never_names_nyxbot_and_links_the_agent_page() {
    for area in [
        AccessArea::Service,
        AccessArea::Account,
        AccessArea::Operations,
        AccessArea::Skills,
        AccessArea::Machine,
        AccessArea::SavedLogin,
        AccessArea::MachineCapabilities,
        AccessArea::Connect,
        AccessArea::Schedules,
    ] {
        let value = missing_access(area, Some("GitHub"), "agent-1", None);
        let text = value.to_string().to_lowercase();
        assert!(!text.contains("nyxbot"), "{area:?}: {value}");
        assert_eq!(value["error"], MISSING_ACCESS);
        assert_eq!(value["scope"], area.scope());
        assert_eq!(value["request_created"], false);
        assert!(
            value["instructions"]
                .as_str()
                .unwrap()
                .contains("No request was created"),
            "{area:?}"
        );
        let path = value["grants_path"].as_str().unwrap();
        assert!(path.contains("agent-1"), "{area:?}: {path}");
        match area {
            AccessArea::Skills => assert!(path.contains("details=skills"), "{path}"),
            AccessArea::Schedules => assert!(path.starts_with("/assistant/automations"), "{path}"),
            _ => assert!(path.contains("details=grants"), "{path}"),
        }
        assert_eq!(AccessArea::from_scope(area.scope()), Some(area));
        let upgraded = with_frontend(value, "https://app.example/");
        assert!(
            upgraded["grants_url"]
                .as_str()
                .unwrap()
                .starts_with("https://app.example/assistant")
        );
    }
    // The machine permission guidance is contextual; only involved work and
    // NyxBot itself hear about NyxBot.
    let direct = machine_permission_guidance(
        &chat(AgentRole::Subagent, false, None, false),
        "https://app.example",
    );
    assert!(!direct.to_lowercase().contains("nyxbot"), "{direct}");
    assert!(direct.contains("No request was created") || direct.contains("no request was created"));
    let involved = machine_permission_guidance(
        &chat(AgentRole::Subagent, false, Some("t"), false),
        "https://app.example",
    );
    assert!(involved.contains("nyxid__machine_capabilities"));
    let guest = machine_permission_guidance(
        &chat(AgentRole::Subagent, true, Some("t"), false),
        "https://app.example",
    );
    assert!(!guest.to_lowercase().contains("nyxbot"));
    assert_eq!(
        connect_refusal(&chat(AgentRole::Subagent, false, None, false))["error"],
        MISSING_ACCESS
    );
    assert_eq!(
        connect_refusal(&chat(AgentRole::Subagent, false, Some("t"), false)),
        acks::access_request_denied()
    );
    // The neutral error text never names NyxBot either.
    assert!(
        !crate::errors::AppError::MachinePermissionRevoked
            .to_string()
            .contains("NyxBot")
    );
}

#[test]
fn error_result_restores_the_exact_contextual_missing_access_value() {
    let value = missing_access(AccessArea::Service, Some("GitHub"), "agent-1", None);
    let error = missing_access_error(&value);
    assert!(matches!(error, AppError::AssistantMissingAccess(_)));
    assert!(error.is_forbidden());
    assert_eq!(error.error_code(), 1002);
    assert_eq!(error.error_key(), "missing_access");
    assert!(!error.to_string().to_lowercase().contains("nyxbot"));
    assert_eq!(error.response_body().details, Some(value.clone()));
    let result = tools::error_result(error);
    assert!(result.is_error);
    assert_eq!(result.value, value);
    // A plain forbidden error is never rebuilt into a missing-access value.
    let plain = tools::error_result(AppError::Forbidden("missing_access: nope".into()));
    assert_ne!(plain.value["error"], MISSING_ACCESS);
}

#[test]
fn cutover_marker_parsing_fails_closed() {
    assert!(parse_cutover(&doc! {"_id": CUTOVER_MARKER_ID}).is_err());
    assert!(parse_cutover(&doc! {"_id": CUTOVER_MARKER_ID, "cutover_at": "2026-10-15"}).is_err());
    let at = Utc::now();
    assert_eq!(
        parse_cutover(&doc! {"cutover_at": bson::DateTime::from_chrono(at)})
            .unwrap()
            .timestamp_millis(),
        at.timestamp_millis()
    );
}

#[tokio::test]
async fn cutover_marker_is_written_once_and_never_moves() {
    let f = fixture("policy_cutover_marker").await;
    let db = &f.state.db;
    let first = ensure_cutover(db).await.unwrap();
    let stored = stored_cutover(db).await.unwrap().expect("marker written");
    // Another replica's startup (the same $setOnInsert, later) keeps the
    // original value; so does a repeated call here.
    db.collection::<Document>(MIGRATIONS_COLLECTION_NAME)
        .update_one(
            doc! {"_id": CUTOVER_MARKER_ID},
            doc! {"$setOnInsert": {"cutover_at": bson::DateTime::from_chrono(first + Duration::hours(2))}},
        )
        .upsert(true)
        .await
        .unwrap();
    assert_eq!(
        stored_cutover(db)
            .await
            .unwrap()
            .unwrap()
            .timestamp_millis(),
        stored.timestamp_millis()
    );
    assert_eq!(ensure_cutover(db).await.unwrap(), first);
    assert_eq!(cutover_at(db), Some(first));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn direct_specialist_work_gets_missing_access_without_card_or_nyxbot_notice() {
    let f = direct_fixture("policy_direct_missing_access").await;
    let db = &f.state.db;
    assert!(!f.chat.involved());
    assert!(f.chat.turn_live && f.chat.report_to.is_none() && !f.chat.with_nyxbot);
    let service = connected(db, &f.owner, "github", "https://api.github.com").await;
    let mut results = Vec::new();
    let (value, card) = acks::service_gate(db, &f.chat, &service, "github", "GitHub", false)
        .await
        .unwrap()
        .unwrap();
    assert!(card.is_none());
    assert_eq!(value["target"], "GitHub");
    results.push(("service", value));
    let (value, card) = acks::account_gate(db, &f.chat).await.unwrap().unwrap();
    assert!(card.is_none());
    results.push(("account", value));
    for (kind, scope, tool) in [
        ("operations", "operations", None),
        ("machine", "machine", None),
        ("saved_login", "saved_login", None),
        (
            "machine",
            "machine_capabilities",
            Some("nyxid__machine_capabilities"),
        ),
    ] {
        let target = Uuid::new_v4().to_string();
        let error = acks::request_tracked(db, &f.chat, service_request(&target, kind, tool))
            .await
            .unwrap_err();
        let result = tools::error_result(error);
        assert!(result.is_error, "{kind}");
        results.push((scope, result.value));
    }
    let (value, error) = crate::handlers::agent_skills::dispatch(
        &f.state,
        &f.chat,
        "request_agent_skills",
        &json!({"agent": f.chat.agent_id, "skill": "weekly-review", "version": "1.0.0"}),
    )
    .await
    .unwrap();
    assert!(error);
    assert_eq!(value["status"], "not_attached");
    results.push(("skills", value));
    let (value, error) = crate::handlers::agent_skills::dispatch(
        &f.state,
        &f.chat,
        "request_agent_skills",
        &json!({"agent": f.chat.agent_id, "selection": {"expected_revision": 0, "skills": [{
            "source": "ornn", "skill_id": Uuid::new_v4().to_string(), "name": "weekly-review",
            "version": "1.0", "sha256": "a".repeat(64), "dependencies": []}]}}),
    )
    .await
    .unwrap();
    assert!(error);
    results.push(("skills", value));
    let error = crate::handlers::machine_tools::permission(
        &f.state,
        &f.chat,
        "saved_login",
        &Uuid::new_v4().to_string(),
        "Mail login",
    )
    .await
    .unwrap_err();
    results.push(("saved_login", tools::error_result(error).value));
    for (scope, raw) in &results {
        // Both typed refusals and successful gate values reach this same
        // final assistant boundary before becoming model context.
        let result = tools::error_result_for(
            missing_access_error(raw),
            Some(&f.chat),
            &f.state.config.frontend_url,
        );
        let value = &result.value;
        let instructions = value["instructions"].as_str().unwrap();
        assert!(
            instructions.contains(&format!(
                "({}/assistant",
                f.state.config.frontend_url.trim_end_matches('/')
            )),
            "{scope}: {instructions}"
        );
        assert!(
            !instructions.contains("(/assistant?"),
            "{scope}: {instructions}"
        );
        assert_eq!(
            with_frontend(value.clone(), &f.state.config.frontend_url),
            *value
        );
        assert_eq!(value["error"], MISSING_ACCESS, "{scope}: {value}");
        assert_eq!(value["scope"], *scope, "{value}");
        assert_eq!(value["request_created"], false, "{scope}");
        assert_eq!(value["agent_id"], f.chat.agent_id, "{scope}");
        assert!(
            value["grants_path"]
                .as_str()
                .unwrap()
                .contains(&f.chat.agent_id),
            "{scope}"
        );
        assert!(
            !value.to_string().to_lowercase().contains("nyxbot"),
            "{scope}: {value}"
        );
        assert!(
            value["instructions"]
                .as_str()
                .unwrap()
                .contains("No request was created"),
            "{scope}"
        );
    }
    // No card of any kind, no NyxBot event, message or outbox entry.
    assert_eq!(card_count(db, doc! {}).await, 0);
    let nyxbot = engine::get(db, &f.owner, &f.nyxbot_thread).await.unwrap();
    assert!(nyxbot.pending_events.is_empty());
    assert_eq!(denial_notices(db, &f.owner, &f.nyxbot_thread).await, 0);
    assert_eq!(
        db.collection::<Document>(ACKS)
            .count_documents(doc! {"skill_notification_pending": true})
            .await
            .unwrap(),
        0
    );
    // Every refusal is audited as metadata, marked as missing access.
    let audits = db
        .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .count_documents(doc! {"event_type": "assistant_access_request_denied",
        "event_data.decision": "missing_access", "event_data.involved": false,
        "event_data.requester_agent": &f.chat.agent_id})
        .await
        .unwrap();
    assert_eq!(audits as usize, results.len());
    assert_eq!(
        db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type": "assistant_access_request_denied",
            "event_data.decision": "denied"})
            .await
            .unwrap(),
        0
    );
    // Denied or routed, the owner's setting changes nothing for direct work.
    settings::update(db, &f.owner, denied_mode()).await.unwrap();
    let (value, card) = acks::service_gate(db, &f.chat, &service, "github", "GitHub", false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(value["error"], MISSING_ACCESS);
    assert!(card.is_none());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn involved_work_keeps_denied_notice_to_the_exact_assigning_thread_and_routed_cards() {
    let f = fixture("policy_involved_exact_target").await;
    let db = &f.state.db;
    assert!(f.chat.involved());
    assert_eq!(f.chat.report_to.as_deref(), Some(f.nyxbot_thread.as_str()));
    settings::update(db, &f.owner, denied_mode()).await.unwrap();
    let service = connected(db, &f.owner, "github", "https://api.github.com").await;
    let (value, card) = acks::service_gate(db, &f.chat, &service, "github", "GitHub", false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(value, acks::access_request_denied());
    assert!(card.is_none());
    assert_eq!(denial_notices(db, &f.owner, &f.nyxbot_thread).await, 1);
    let nyxbot = engine::get(db, &f.owner, &f.nyxbot_thread).await.unwrap();
    assert!(
        nyxbot
            .pending_events
            .iter()
            .any(|event| event.kind == "permission_requested" && !event.bound)
    );
    // Work assigned from another NyxBot thread notifies that thread, not home.
    let other = new_orchestrator(&f.state, &f.owner, "Other assignment").await;
    let row = pin_assignment(db, &f.owner, &f.row.id, Some(&other.id)).await;
    let chat = chat_for_row(&f.state, &f.owner, &row).await;
    assert_eq!(chat.report_to.as_deref(), Some(other.id.as_str()));
    let (value, _) = acks::service_gate(db, &chat, &service, "github", "GitHub", false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(value, acks::access_request_denied());
    assert_eq!(denial_notices(db, &f.owner, &other.id).await, 1);
    assert_eq!(denial_notices(db, &f.owner, &f.nyxbot_thread).await, 1);
    // Routed mode: the card stores the exact routing and NyxBot may decide it.
    settings::update(db, &f.owner, routed_mode()).await.unwrap();
    let (value, card) = acks::service_gate(db, &chat, &service, "github", "GitHub", false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(value["error"], "acknowledgement_required");
    let card = card.expect("routed request created");
    assert_eq!(
        card.routing,
        Some(CardRouting::Assigned {
            report_to: other.id.clone()
        })
    );
    assert_eq!(
        nyxbot_destination(db, &f.owner, card.routing.as_ref(), None)
            .await
            .unwrap()
            .as_deref(),
        Some(other.id.as_str())
    );
    let decided = acks::decide_as(
        db,
        &f.owner,
        None,
        &card.id,
        true,
        acks::Decider::Nyxbot,
        Some("the user asked for it"),
    )
    .await
    .unwrap();
    assert_eq!(decided.status, "allowed");
    // An assignment naming a non-NyxBot thread is no destination at all.
    let bogus = CardRouting::Assigned {
        report_to: f.row.id.clone(),
    };
    assert_eq!(
        nyxbot_destination(db, &f.owner, Some(&bogus), None)
            .await
            .unwrap(),
        None
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pending_cards_deduplicate_only_within_the_same_routing() {
    let f = fixture("policy_dedupe_per_routing").await;
    let db = &f.state.db;
    let service = connected(db, &f.owner, "github", "https://api.github.com").await;
    let (first, created) = acks::service_gate(db, &f.chat, &service, "github", "GitHub", false)
        .await
        .unwrap()
        .unwrap();
    let first_card = created.expect("first request");
    let other = new_orchestrator(&f.state, &f.owner, "Other assignment").await;
    let row = pin_assignment(db, &f.owner, &f.row.id, Some(&other.id)).await;
    let chat = chat_for_row(&f.state, &f.owner, &row).await;
    let (second, created) = acks::service_gate(db, &chat, &service, "github", "GitHub", false)
        .await
        .unwrap()
        .unwrap();
    let second_card = created.expect("a card for the other assignment");
    assert_ne!(first_card.id, second_card.id);
    assert_ne!(first["acknowledgement_id"], second["acknowledgement_id"]);
    assert_eq!(
        second_card.routing,
        Some(CardRouting::Assigned {
            report_to: other.id.clone()
        })
    );
    // Back on the first assignment, the first card is reused.
    let row = pin_assignment(db, &f.owner, &f.row.id, Some(&f.nyxbot_thread)).await;
    let chat = chat_for_row(&f.state, &f.owner, &row).await;
    let (again, created) = acks::service_gate(db, &chat, &service, "github", "GitHub", false)
        .await
        .unwrap()
        .unwrap();
    assert!(created.is_none());
    assert_eq!(again["acknowledgement_id"], first_card.id);
    assert_eq!(card_count(db, doc! {"kind": "service"}).await, 2);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_decides_only_cards_raised_for_it_while_the_owner_decides_every_card() {
    let f = fixture("policy_nyxbot_decides_its_cards").await;
    let db = &f.state.db;
    let service = connected(db, &f.owner, "github", "https://api.github.com").await;
    let (_, card) = acks::service_gate(db, &f.chat, &service, "github", "GitHub", false)
        .await
        .unwrap()
        .unwrap();
    let card = card.unwrap();
    let cards = db.collection::<AssistantAcknowledgement>(ACKS);
    let mut legacy = card.clone();
    legacy.id = Uuid::new_v4().to_string();
    legacy.routing = None;
    cards.insert_one(&legacy).await.unwrap();
    let mut alone = card.clone();
    alone.id = Uuid::new_v4().to_string();
    alone.routing = Some(CardRouting::NotInvolved);
    cards.insert_one(&alone).await.unwrap();
    for id in [&legacy.id, &alone.id] {
        let error = acks::decide_as(db, &f.owner, None, id, true, acks::Decider::Nyxbot, None)
            .await
            .unwrap_err();
        assert!(error.is_forbidden(), "{id}: {error}");
        let row = cards.find_one(doc! {"_id": id}).await.unwrap().unwrap();
        assert_eq!(row.status, "pending");
    }
    // The owner keeps authority over legacy and not-involved cards.
    let decided = acks::decide(db, &f.owner, &f.row.id, &legacy.id, true)
        .await
        .unwrap();
    assert_eq!(decided.status, "allowed");
    let decided = acks::decide(db, &f.owner, &f.row.id, &alone.id, false)
        .await
        .unwrap();
    assert_eq!(decided.status, "denied");
    // The assigned card is NyxBot's to decide.
    let decided = acks::decide_as(
        db,
        &f.owner,
        None,
        &card.id,
        true,
        acks::Decider::Nyxbot,
        None,
    )
    .await
    .unwrap();
    assert_eq!(decided.status, "allowed");
    assert_eq!(decided.decided_by.as_deref(), Some("orchestrator"));
    f.state.db.drop().await.unwrap();
}

async fn personal_group(db: &Database, owner: &str, members: &[&str]) -> String {
    let id = Uuid::new_v4().to_string();
    let lead = members[0];
    db.collection::<Document>(crate::models::assistant_group::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": &id, "user_id": owner, "name": "Room",
            "member_agent_ids": members.to_vec(), "lead_agent_id": lead,
            "created_by": "user", "message_count": 0_i64,
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn group_work_with_nyxbot_routes_to_its_home_and_rechecks_membership() {
    let f = fixture("policy_group_with_nyxbot").await;
    let db = &f.state.db;
    let nyxbot = team::ensure_nyxbot(db, &f.owner).await.unwrap();
    let home = nyxbot.home_conversation_id.clone().unwrap();
    let group = personal_group(db, &f.owner, &[&nyxbot.id, &f.chat.agent_id]).await;
    let row = pin_group_with_nyxbot(db, &f.owner, &f.row.id, &group).await;
    let chat = chat_for_row(&f.state, &f.owner, &row).await;
    assert!(chat.involved() && chat.with_nyxbot && chat.report_to.is_none());
    assert!(
        nyxbot_in_personal_group(db, &f.owner, &group, None)
            .await
            .unwrap()
    );
    let service = connected(db, &f.owner, "github", "https://api.github.com").await;
    let (value, card) = acks::service_gate(db, &chat, &service, "github", "GitHub", false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(value["error"], "acknowledgement_required");
    let card = card.unwrap();
    assert_eq!(
        card.routing,
        Some(CardRouting::GroupWithNyxbot {
            group_id: group.clone()
        })
    );
    assert_eq!(
        nyxbot_destination(db, &f.owner, card.routing.as_ref(), None)
            .await
            .unwrap()
            .as_deref(),
        Some(home.as_str())
    );
    // A denied-mode notice for group work goes to NyxBot's home.
    settings::update(db, &f.owner, denied_mode()).await.unwrap();
    let other = connected(db, &f.owner, "docs", "https://docs.example").await;
    let (value, _) = acks::service_gate(db, &chat, &other, "docs", "Docs", false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(value, acks::access_request_denied());
    assert_eq!(denial_notices(db, &f.owner, &home).await, 1);
    // NyxBot leaves the group: its decision conflicts, nothing routes, the
    // owner still decides.
    db.collection::<Document>(crate::models::assistant_group::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &group},
            doc! {"$pull": {"member_agent_ids": &nyxbot.id}},
        )
        .await
        .unwrap();
    let error = acks::decide_as(
        db,
        &f.owner,
        None,
        &card.id,
        true,
        acks::Decider::Nyxbot,
        None,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)), "{error}");
    assert_eq!(
        nyxbot_destination(db, &f.owner, card.routing.as_ref(), None)
            .await
            .unwrap(),
        None
    );
    let decided = acks::decide(db, &f.owner, &f.row.id, &card.id, true)
        .await
        .unwrap();
    assert_eq!(decided.status, "allowed");
    // An org group never counts as a group NyxBot is in.
    let org_group = personal_group(db, &f.owner, &[&nyxbot.id]).await;
    db.collection::<Document>(crate::models::assistant_group::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &org_group},
            doc! {"$set": {"created_by_user_id": &f.owner}},
        )
        .await
        .unwrap();
    assert!(
        !nyxbot_in_personal_group(db, &f.owner, &org_group, None)
            .await
            .unwrap()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn allowed_skill_shortcut_applies_only_within_the_same_routing() {
    let f = fixture("policy_skill_shortcut_routing").await;
    let db = &f.state.db;
    let args = json!({"agent": f.chat.agent_id, "skill": "weekly-review", "version": "1.0.0"});
    let request = |chat: ChatAuthority| {
        let state = f.state.clone();
        let args = args.clone();
        async move {
            crate::handlers::agent_skills::dispatch(&state, &chat, "request_agent_skills", &args)
                .await
                .unwrap()
        }
    };
    let (value, error) = request(f.chat.clone()).await;
    assert!(error);
    let id = value["acknowledgement_id"].as_str().unwrap().to_owned();
    let card = db
        .collection::<AssistantAcknowledgement>(ACKS)
        .find_one(doc! {"_id": &id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        card.routing,
        Some(CardRouting::Assigned {
            report_to: f.nyxbot_thread.clone()
        })
    );
    acks::decide(db, &f.owner, &f.row.id, &id, true)
        .await
        .unwrap();
    let (again, error) = request(f.chat.clone()).await;
    assert!(!error, "{again}");
    assert_eq!(again["status"], "allowed_pending_attachment");
    assert_eq!(again["request_id"], id);
    assert!(!again["instructions"].as_str().unwrap().contains("NyxBot"));
    // The same request from work assigned elsewhere is a new card.
    let other = new_orchestrator(&f.state, &f.owner, "Other assignment").await;
    let row = pin_assignment(db, &f.owner, &f.row.id, Some(&other.id)).await;
    let (fresh, error) = request(chat_for_row(&f.state, &f.owner, &row).await).await;
    assert!(error);
    assert_ne!(fresh["acknowledgement_id"], id);
    assert_eq!(fresh["error"], "acknowledgement_required");
    // Direct work reuses nothing and raises nothing.
    let before = card_count(db, doc! {}).await;
    let row = pin_assignment(db, &f.owner, &f.row.id, None).await;
    let direct = chat_for_row(&f.state, &f.owner, &row).await;
    let (value, error) = request(direct.clone()).await;
    assert!(error);
    assert_eq!(value["error"], MISSING_ACCESS);
    assert_eq!(value["status"], "not_attached");
    assert_eq!(card_count(db, doc! {}).await, before);
    // Once attached, the skill is simply usable.
    db.collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.chat.agent_id},
            doc! {"$set": {"skills": [{
            "source": "ornn", "skill_id": Uuid::new_v4().to_string(), "name": "weekly-review",
            "version": "1.0.0", "sha256": "a".repeat(64), "dependencies": []}]}},
        )
        .await
        .unwrap();
    let (value, error) = request(direct).await;
    assert!(!error, "{value}");
    assert_eq!(value["status"], "already_attached");
    assert_eq!(value["request_created"], false);
    assert!(value.get("error").is_none());
    assert!(
        value["instructions"]
            .as_str()
            .unwrap()
            .contains("nyxid__skill_read")
    );
    assert!(!value.to_string().to_lowercase().contains("nyxbot"));
    assert_eq!(card_count(db, doc! {}).await, before);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn requests_and_notices_need_the_exact_live_turn_and_snapshot() {
    let f = fixture("policy_turn_fences").await;
    let db = &f.state.db;
    settings::update(db, &f.owner, denied_mode()).await.unwrap();
    let conversations =
        db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME);
    let stale = f.chat.clone();
    let turn_id = stale.turn_id.clone().unwrap();
    let service = Uuid::new_v4().to_string();
    let set = |update: Document| {
        let conversations = conversations.clone();
        let id = f.row.id.clone();
        async move {
            conversations
                .update_one(doc! {"_id": id}, doc! {"$set": update})
                .await
                .unwrap();
        }
    };
    for (name, change) in [
        ("stopped", doc! {"active_turn.stop_requested": true}),
        (
            "replaced",
            doc! {"active_turn.stop_requested": false, "active_turn.turn_id": Uuid::new_v4().to_string()},
        ),
        (
            "expired",
            doc! {"active_turn.turn_id": &turn_id,
            "active_turn.lease_expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::minutes(1))},
        ),
        (
            "reassigned",
            doc! {"active_turn.lease_expires_at": bson::DateTime::from_chrono(Utc::now() + Duration::minutes(30)),
            "active_turn.report_to": Uuid::new_v4().to_string()},
        ),
    ] {
        set(change).await;
        settings::update(db, &f.owner, routed_mode()).await.unwrap();
        let error = acks::request_tracked(db, &stale, service_request(&service, "service", None))
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)), "{name}: {error}");
        settings::update(db, &f.owner, denied_mode()).await.unwrap();
        acks::audit_access_request_denied(db, &stale, Some(&service), "service").await;
        assert_eq!(
            denial_notices(db, &f.owner, &f.nyxbot_thread).await,
            0,
            "{name}"
        );
    }
    assert_eq!(card_count(db, doc! {"kind": "service"}).await, 0);
    // Restored, the same chat raises its request and notice again.
    set(doc! {"active_turn.report_to": &f.nyxbot_thread}).await;
    acks::audit_access_request_denied(db, &stale, Some(&service), "service").await;
    assert_eq!(denial_notices(db, &f.owner, &f.nyxbot_thread).await, 1);
    settings::update(db, &f.owner, routed_mode()).await.unwrap();
    let (_, created) =
        acks::request_tracked(db, &stale, service_request(&service, "service", None))
            .await
            .unwrap();
    assert!(created);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn chat_authority_reads_the_snapshot_only_from_a_live_unstopped_turn() {
    let f = fixture("policy_snapshot_live_only").await;
    let db = &f.state.db;
    let conversations =
        db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME);
    let reload = || chat_for_row(&f.state, &f.owner, &f.row);
    assert!(reload().await.involved());
    conversations
        .update_one(
            doc! {"_id": &f.row.id},
            doc! {"$set": {"active_turn.stop_requested": true}},
        )
        .await
        .unwrap();
    let stopped = reload().await;
    assert!(!stopped.involved() && stopped.report_to.is_none());
    conversations
        .update_one(
            doc! {"_id": &f.row.id},
            doc! {"$set": {"active_turn.stop_requested": false,
                "active_turn.lease_expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::minutes(1))}},
        )
        .await
        .unwrap();
    let expired = reload().await;
    assert!(!expired.involved() && expired.report_to.is_none());
    conversations
        .update_one(
            doc! {"_id": &f.row.id},
            doc! {"$unset": {"active_turn": ""}},
        )
        .await
        .unwrap();
    let idle = reload().await;
    assert!(!idle.involved() && idle.report_to.is_none() && !idle.with_nyxbot);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn admission_filters_ineligible_skill_events_and_terminalizes_their_outbox() {
    let f = fixture("policy_admission_filter").await;
    let db = &f.state.db;
    let cutover = ensure_cutover(db).await.unwrap();
    assert_eq!(cutover_at(db), Some(cutover));
    assert_eq!(ensure_cutover(db).await.unwrap(), cutover);
    let cards = db.collection::<Document>(ACKS);
    let mut ids = Vec::new();
    for skill in ["alpha", "beta", "gamma", "delta", "epsilon", "zeta"] {
        let (value, error) = crate::handlers::agent_skills::dispatch(
            &f.state,
            &f.chat,
            "request_agent_skills",
            &json!({"agent": f.chat.agent_id, "skill": skill}),
        )
        .await
        .unwrap();
        assert!(error, "{value}");
        let id = value["acknowledgement_id"].as_str().unwrap().to_owned();
        acks::decide(db, &f.owner, &f.row.id, &id, true)
            .await
            .unwrap();
        ids.push(id);
    }
    let [assigned, alone, legacy, stripped, group_gone, group_live] = ids.as_slice() else {
        unreachable!()
    };
    let group = {
        let nyxbot = team::ensure_nyxbot(db, &f.owner).await.unwrap();
        personal_group(db, &f.owner, &[&nyxbot.id, &f.chat.agent_id]).await
    };
    for (id, update) in [
        (alone, doc! {"$set": {"routing": {"kind": "not_involved"}}}),
        (
            legacy,
            doc! {"$unset": {"routing": ""}, "$set": {"created_at": bson::DateTime::from_chrono(cutover - Duration::hours(1))}},
        ),
        (
            stripped,
            doc! {"$unset": {"routing": ""}, "$set": {"created_at": bson::DateTime::from_chrono(cutover + Duration::seconds(1))}},
        ),
        (
            group_gone,
            doc! {"$set": {"routing": {"kind": "group_with_nyxbot", "group_id": Uuid::new_v4().to_string()}}},
        ),
        (
            group_live,
            doc! {"$set": {"routing": {"kind": "group_with_nyxbot", "group_id": &group}}},
        ),
    ] {
        cards.update_one(doc! {"_id": id}, update).await.unwrap();
    }
    assert_eq!(
        cards
            .count_documents(doc! {"skill_notification_pending": true})
            .await
            .unwrap(),
        6
    );
    let mut events: Vec<_> = ids
        .iter()
        .map(|id| {
            let mut event = team::event(
                "skill_request_allowed",
                "allowed".into(),
                Some(&f.chat.agent_id),
            );
            event.id = id.clone();
            event
        })
        .collect();
    events.push(team::event("message", "owner said hi".into(), None));
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let admitted = admit_skill_events_in_session(db, &f.owner, events, &mut session)
        .await
        .unwrap();
    session.commit_transaction().await.unwrap();
    let eligible: Vec<_> = admitted.eligible.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(eligible.len(), 4);
    assert!(eligible.contains(&assigned.as_str()));
    assert!(eligible.contains(&legacy.as_str()));
    assert!(eligible.contains(&group_live.as_str()));
    assert_eq!(admitted.eligible.last().unwrap().kind, "message");
    assert_eq!(admitted.dropped.len(), 3);
    for (id, reason) in [
        (alone, "not_involved"),
        (stripped, "unbound_after_cutover"),
        (group_gone, "group_membership_changed"),
    ] {
        assert!(admitted.dropped.contains(id), "{reason}");
        let row = cards.find_one(doc! {"_id": id}).await.unwrap().unwrap();
        assert!(
            row.get_bool("skill_notification_pending").is_err(),
            "{reason}"
        );
        assert_eq!(
            row.get_document("skill_notification_terminal")
                .unwrap()
                .get_str("reason")
                .unwrap(),
            reason
        );
    }
    for id in [assigned, legacy, group_live] {
        let row = cards.find_one(doc! {"_id": id}).await.unwrap().unwrap();
        assert!(row.get_bool("skill_notification_pending").is_err());
        assert!(row.get_document("skill_notification_terminal").is_err());
    }
    // Nothing is left for the sweep to retry.
    assert!(
        acks::skill_notifications::pending(db)
            .await
            .unwrap()
            .is_empty()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn skill_notification_delivery_follows_the_cards_routing() {
    let f = fixture("policy_skill_delivery_routing").await;
    let db = &f.state.db;
    ensure_cutover(db).await.unwrap();
    let request = |skill: &str| {
        let state = f.state.clone();
        let chat = f.chat.clone();
        let args = json!({"agent": f.chat.agent_id, "skill": skill});
        async move {
            let (value, _) = crate::handlers::agent_skills::dispatch(
                &state,
                &chat,
                "request_agent_skills",
                &args,
            )
            .await
            .unwrap();
            value["acknowledgement_id"].as_str().unwrap().to_owned()
        }
    };
    let cards = db.collection::<Document>(ACKS);
    // A card stripped of its routing after the cutover never reaches NyxBot.
    let stripped = request("stripped").await;
    cards
        .update_one(doc! {"_id": &stripped}, doc! {"$unset": {"routing": ""}})
        .await
        .unwrap();
    let card = acks::decide(db, &f.owner, &f.row.id, &stripped, true)
        .await
        .unwrap();
    crate::handlers::assistant_team::permission_decided(&f.state, &f.owner, &card).await;
    let row = cards
        .find_one(doc! {"_id": &stripped})
        .await
        .unwrap()
        .unwrap();
    assert!(row.get_bool("skill_notification_pending").is_err());
    assert_eq!(
        row.get_document("skill_notification_terminal")
            .unwrap()
            .get_str("reason")
            .unwrap(),
        "unbound_after_cutover"
    );
    let nyxbot = engine::get(db, &f.owner, &f.nyxbot_thread).await.unwrap();
    assert!(
        nyxbot
            .pending_events
            .iter()
            .all(|event| event.id != stripped)
    );
    // The specialist's own continuation is bound without a report target.
    let specialist = engine::get(db, &f.owner, &f.row.id).await.unwrap();
    let decided = specialist
        .pending_events
        .iter()
        .find(|event| event.kind == "permission_decided")
        .expect("decision continuation");
    assert!(!decided.bound && decided.report_to.is_none());
    // An assigned card reaches the exact assigning thread, and its decision
    // continuation keeps reporting there.
    let assigned = request("assigned").await;
    let card = acks::decide(db, &f.owner, &f.row.id, &assigned, true)
        .await
        .unwrap();
    assert_eq!(
        card.routing,
        Some(CardRouting::Assigned {
            report_to: f.nyxbot_thread.clone()
        })
    );
    crate::handlers::assistant_team::permission_decided(&f.state, &f.owner, &card).await;
    let nyxbot = engine::get(db, &f.owner, &f.nyxbot_thread).await.unwrap();
    assert!(
        nyxbot
            .pending_events
            .iter()
            .any(|event| event.id == assigned && event.kind == "skill_request_allowed")
    );
    let specialist = engine::get(db, &f.owner, &f.row.id).await.unwrap();
    let decided = specialist
        .pending_events
        .iter()
        .rfind(|event| event.kind == "permission_decided")
        .unwrap();
    assert!(decided.bound);
    assert_eq!(decided.report_to.as_deref(), Some(f.nyxbot_thread.as_str()));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn routed_request_notice_is_fenced_to_the_live_turn_and_the_cards_routing() {
    let f = fixture("policy_routed_notice_fence").await;
    let db = &f.state.db;
    let conversations =
        db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME);
    let service = connected(db, &f.owner, "github", "https://api.github.com").await;
    let (_, card) = acks::service_gate(db, &f.chat, &service, "github", "GitHub", false)
        .await
        .unwrap()
        .unwrap();
    let card = card.expect("routed request");
    let event = || {
        team::event(
            "permission_requested",
            "needs GitHub".into(),
            Some(&f.chat.agent_id),
        )
    };
    let queued = |thread: String, request_id: String| {
        let state = f.state.clone();
        let owner = f.owner.clone();
        async move {
            engine::get(&state.db, &owner, &thread)
                .await
                .unwrap()
                .pending_events
                .iter()
                .filter(|e| e.id == acks::permission_requested_event_id(&request_id))
                .count()
        }
    };
    // Live assigned turn: queued once on the exact assigning thread, idempotently.
    assert_eq!(
        acks::notify_nyxbot_request(db, &f.chat, &card, event())
            .await
            .unwrap()
            .as_deref(),
        Some(f.nyxbot_thread.as_str())
    );
    assert_eq!(
        acks::notify_nyxbot_request(db, &f.chat, &card, event())
            .await
            .unwrap()
            .as_deref(),
        Some(f.nyxbot_thread.as_str())
    );
    assert_eq!(queued(f.nyxbot_thread.clone(), card.id.clone()).await, 1);
    let kept = engine::get(db, &f.owner, &f.nyxbot_thread)
        .await
        .unwrap()
        .pending_events
        .into_iter()
        .find(|e| e.id == acks::permission_requested_event_id(&card.id))
        .unwrap();
    assert!(!kept.bound && kept.report_to.is_none());
    // Stop, replacement and lease expiry after authentication: no notice.
    let turn_id = f.chat.turn_id.clone().unwrap();
    let other = new_orchestrator(&f.state, &f.owner, "Other").await;
    let second = connected(db, &f.owner, "docs", "https://docs.example").await;
    let (_, second_card) = acks::service_gate(db, &f.chat, &second, "docs", "Docs", false)
        .await
        .unwrap()
        .unwrap();
    let second_card = second_card.expect("second routed request");
    for (name, change) in [
        ("stopped", doc! {"active_turn.stop_requested": true}),
        (
            "replaced",
            doc! {"active_turn.stop_requested": false, "active_turn.turn_id": Uuid::new_v4().to_string()},
        ),
        (
            "expired",
            doc! {"active_turn.turn_id": &turn_id,
            "active_turn.lease_expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::minutes(1))},
        ),
        (
            "reassigned",
            doc! {"active_turn.lease_expires_at": bson::DateTime::from_chrono(Utc::now() + Duration::minutes(30)),
            "active_turn.report_to": &other.id},
        ),
    ] {
        conversations
            .update_one(doc! {"_id": &f.row.id}, doc! {"$set": change})
            .await
            .unwrap();
        let error = acks::notify_nyxbot_request(db, &f.chat, &second_card, event())
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)), "{name}: {error}");
        assert_eq!(
            queued(f.nyxbot_thread.clone(), second_card.id.clone()).await,
            0,
            "{name}"
        );
        assert_eq!(
            queued(other.id.clone(), second_card.id.clone()).await,
            0,
            "{name}"
        );
    }
    conversations
        .update_one(
            doc! {"_id": &f.row.id},
            doc! {"$set": {"active_turn.report_to": &f.nyxbot_thread}},
        )
        .await
        .unwrap();
    // A card decided meanwhile needs no notice.
    acks::decide(db, &f.owner, &f.row.id, &second_card.id, false)
        .await
        .unwrap();
    assert_eq!(
        acks::notify_nyxbot_request(db, &f.chat, &second_card, event())
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        queued(f.nyxbot_thread.clone(), second_card.id.clone()).await,
        0
    );
    // A legacy card without routing never tells NyxBot.
    let third = connected(db, &f.owner, "wiki", "https://wiki.example").await;
    let (_, legacy) = acks::service_gate(db, &f.chat, &third, "wiki", "Wiki", false)
        .await
        .unwrap()
        .unwrap();
    let mut legacy = legacy.expect("third routed request");
    db.collection::<Document>(ACKS)
        .update_one(doc! {"_id": &legacy.id}, doc! {"$unset": {"routing": ""}})
        .await
        .unwrap();
    legacy.routing = None;
    assert_eq!(
        acks::notify_nyxbot_request(db, &f.chat, &legacy, event())
            .await
            .unwrap(),
        None
    );
    assert_eq!(queued(f.nyxbot_thread.clone(), legacy.id.clone()).await, 0);
    // Direct work never raises a notice, whatever card is handed in.
    let row = pin_assignment(db, &f.owner, &f.row.id, None).await;
    let direct = chat_for_row(&f.state, &f.owner, &row).await;
    assert_eq!(
        acks::notify_nyxbot_request(db, &direct, &card, event())
            .await
            .unwrap(),
        None
    );
    // Group work: NyxBot's home while NyxBot is a member; nothing once it left.
    let nyxbot = team::ensure_nyxbot(db, &f.owner).await.unwrap();
    let home = nyxbot.home_conversation_id.clone().unwrap();
    let group = personal_group(db, &f.owner, &[&nyxbot.id, &f.chat.agent_id]).await;
    let row = pin_group_with_nyxbot(db, &f.owner, &f.row.id, &group).await;
    let chat = chat_for_row(&f.state, &f.owner, &row).await;
    let fourth = connected(db, &f.owner, "sheets", "https://sheets.example").await;
    let (_, group_card) = acks::service_gate(db, &chat, &fourth, "sheets", "Sheets", false)
        .await
        .unwrap()
        .unwrap();
    let group_card = group_card.expect("group routed request");
    assert_eq!(
        group_card.routing,
        Some(CardRouting::GroupWithNyxbot {
            group_id: group.clone()
        })
    );
    assert_eq!(
        acks::notify_nyxbot_request(db, &chat, &group_card, event())
            .await
            .unwrap()
            .as_deref(),
        Some(home.as_str())
    );
    assert_eq!(queued(home.clone(), group_card.id.clone()).await, 1);
    db.collection::<Document>(crate::models::assistant_group::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &group},
            doc! {"$pull": {"member_agent_ids": &nyxbot.id}},
        )
        .await
        .unwrap();
    let fifth = connected(db, &f.owner, "mail", "https://mail.example").await;
    let (_, left_card) = acks::service_gate(db, &chat, &fifth, "mail", "Mail", false)
        .await
        .unwrap()
        .unwrap();
    let left_card = left_card.expect("card raised before the recheck");
    assert_eq!(
        acks::notify_nyxbot_request(db, &chat, &left_card, event())
            .await
            .unwrap(),
        None
    );
    assert_eq!(queued(home.clone(), left_card.id.clone()).await, 0);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn cutover_cache_is_independent_for_explicit_test_databases() {
    let first = crate::test_utils::connect_transaction_test_database("cutover_first").await;
    let second = crate::test_utils::connect_transaction_test_database("cutover_second").await;
    let early = chrono::DateTime::from_timestamp_millis(1_700_000_000_000).unwrap();
    let late = early + Duration::days(5);
    assert_eq!(initialize_test_cutover(&first, early).await.unwrap(), early);
    assert_eq!(initialize_test_cutover(&second, late).await.unwrap(), late);
    assert_eq!(cutover_at(&first), Some(early));
    assert_eq!(cutover_at(&second), Some(late));
    assert_eq!(ensure_cutover(&first).await.unwrap(), early);
    assert_eq!(ensure_cutover(&second).await.unwrap(), late);
    first.drop().await.unwrap();
    second.drop().await.unwrap();
}

#[test]
fn final_connect_and_revoked_machine_guidance_contains_absolute_links() {
    let chat = chat(AgentRole::Subagent, false, None, false);
    let frontend = "https://nyxid.example/app/";
    let value = with_frontend(connect_refusal(&chat), frontend);
    assert!(
        value["instructions"]
            .as_str()
            .unwrap()
            .contains("(https://nyxid.example/app/assistant?")
    );
    assert!(
        !value["instructions"]
            .as_str()
            .unwrap()
            .contains("(/assistant?")
    );
    let result = tools::error_result_for(AppError::MachinePermissionRevoked, Some(&chat), frontend);
    assert!(
        result.value["instructions"]
            .as_str()
            .unwrap()
            .contains("(https://nyxid.example/app/assistant?")
    );
    assert!(
        !result.value["instructions"]
            .as_str()
            .unwrap()
            .contains("(/assistant?")
    );
}

#[test]
fn specialist_prompt_preserves_granted_service_actions() {
    assert!(engine::SUBAGENT_PROMPT.contains(
        "delete account resources; service actions follow your grants and confirmations"
    ));
    assert!(!engine::SUBAGENT_PROMPT.contains("delete anything"));
}

#[tokio::test]
async fn final_account_and_skill_tool_boundaries_expand_missing_access_instructions() {
    let f = direct_fixture("policy_absolute_tool_results").await;
    let account = Box::pin(
        f.tools()
            .execute(&f.auth, "nyxid__list_my_services", &json!({})),
    )
    .await;
    assert!(account.is_error);
    let (skills, error) = Box::pin(crate::handlers::assistant_team::execute_tool(
        &f.state,
        &f.chat,
        "nyxid__request_agent_skills",
        &json!({"agent": f.chat.agent_id, "skill": "weekly-review", "version": "1.0.0"}),
    ))
    .await;
    assert!(error);
    for value in [account.value, skills] {
        assert_eq!(value["error"], MISSING_ACCESS);
        let instructions = value["instructions"].as_str().unwrap();
        assert!(
            instructions.contains(&format!(
                "({}/assistant?",
                f.state.config.frontend_url.trim_end_matches('/')
            )),
            "{instructions}"
        );
        assert!(!instructions.contains("(/assistant?"));
    }
    f.state.db.drop().await.unwrap();
}
