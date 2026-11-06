//! Real Mongo rollout/compatibility journeys. Legacy consent is never backfilled.
use super::*;
use crate::services::{assistant_nyxagent as engine, assistant_settings_service};

#[derive(Clone, Default)]
pub(super) struct Capture(
    pub(super) Arc<std::sync::Mutex<Vec<std::collections::BTreeMap<String, String>>>>,
);
impl Capture {
    pub(super) fn subscriber(&self) -> tracing::Dispatch {
        use tracing_subscriber::prelude::*;
        tracing::Dispatch::new(tracing_subscriber::registry().with(self.clone()))
    }
}
impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Capture {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        if event.metadata().target() != "nyxid::execution_permission" {
            return;
        }
        struct Fields(std::collections::BTreeMap<String, String>);
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0.insert(field.name().into(), format!("{value:?}"));
            }
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                self.0.insert(field.name().into(), value.into());
            }
        }
        let mut fields = Fields(Default::default());
        event.record(&mut fields);
        assert_eq!(
            *event.metadata().level(),
            if fields.0["decision"] == "legacy" {
                tracing::Level::DEBUG
            } else {
                tracing::Level::INFO
            },
            "legacy routing is debug; participating decisions remain info"
        );
        self.0.lock().unwrap().push(fields.0);
    }
}

async fn count(f: &Fixture, collection: &str) -> u64 {
    f.state
        .db
        .collection::<Document>(collection)
        .count_documents(doc! {})
        .await
        .unwrap()
}

async fn post(f: &Fixture, ack: Option<&str>) -> (StatusCode, serde_json::Value) {
    post_body(f, ack, r#"{"title":"Note"}"#).await
}

async fn post_body(
    f: &Fixture,
    ack: Option<&str>,
    body: &'static str,
) -> (StatusCode, serde_json::Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri("/proxy/s/execution-fixture/status")
        .header("x-api-key", &f.raw)
        .header("content-type", "application/json");
    if let Some(ack) = ack {
        request = request.header(
            super::super::super::assistant_service_execution::ACK_HEADER,
            ack,
        );
    }
    let response = f
        .router()
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn migration_legacy_ack_header_survives_global_enrollment_without_new_card() {
    let mut f = Fixture::new("migration_legacy_ack").await;
    // Only the action card is required; service approval remains an independent layer.
    f.state
        .db
        .collection::<Document>("service_approval_configs")
        .update_many(doc! {}, doc! {"$set":{"approval_required":false}})
        .await
        .unwrap();
    assistant_settings_service::update(
        &f.state.db,
        &f.owner,
        assistant_settings_service::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::AskEveryTime),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mut input = engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        agent_id: None,
        conversation_id: None,
        text: "Update the note".into(),
        model: None,
        access_mode: None,
    };
    let row = engine::begin_turn(&f.state.db, &f.owner, &input, &f.state.encryption_keys)
        .await
        .unwrap();
    f.key = row.credential_api_key_id.clone();
    f.raw = crate::services::assistant_agent_credential_service::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &row.id,
    )
    .await
    .unwrap()
    .unwrap()
    .raw_key
    .to_string();
    assert!(f.auth().await.execution_permission.is_none());
    let (status, card) = post(&f, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{card}");
    let id = card["acknowledgement_id"].as_str().unwrap().to_owned();
    acks::decide(&f.state.db, &f.owner, &row.id, &id, true)
        .await
        .unwrap();
    engine::finish_turn(
        &f.state.db,
        &row,
        &f.key,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "Waiting for review".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    super::super::phase_b_tests::enroll_global(&f.state.db, &f.owner).await;
    input.conversation_id = Some(row.id.clone());
    input.acknowledgement_id = Some(id.clone());
    let mut start: engine::TurnStart = (&input).into();
    start.execution_enrollment = true;
    let resumed = engine::begin_turn(&f.state.db, &f.owner, start, &f.state.encryption_keys)
        .await
        .unwrap();
    assert!(resumed.active_turn.unwrap().execution_task.is_some());
    // A header cannot create authority: unknown IDs and changed arguments
    // stay on the existing exact-card refusal path, without consuming the card.
    for (ack, body) in [
        (Uuid::new_v4().to_string(), r#"{"title":"Note"}"#),
        (id.clone(), r#"{"title":"Changed"}"#),
    ] {
        let (status, refusal) = post_body(&f, Some(&ack), body).await;
        assert_eq!(status, StatusCode::CONFLICT, "{refusal}");
        assert_eq!(refusal["error"], "acknowledgement_invalid");
        assert_eq!(count(&f, "approval_requests").await, 0);
        assert_eq!(count(&f, "assistant_acknowledgements").await, 1);
        assert!(f.server.received_requests().await.unwrap().is_empty());
    }
    let (status, body) = post(&f, Some(&id)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "existing one-use consent must finish without another review: {body}"
    );
    assert_eq!(count(&f, "assistant_acknowledgements").await, 1);
    assert_eq!(count(&f, "approval_requests").await, 0);
    assert_eq!(count(&f, "approval_grants").await, 0);
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    let (status, _) = post(&f, Some(&id)).await;
    assert_ne!(
        status,
        StatusCode::OK,
        "used legacy card must not acquire reusable authority"
    );
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    f.cleanup().await;
}

#[tokio::test]
async fn migration_legacy_grant_scope_owner_expiry_and_storage_are_unchanged() {
    use crate::services::{approval_policy, approval_service};
    let f = Fixture::new("migration_legacy_grants").await;
    f.enroll(true).await;
    f.state
        .db
        .collection::<Document>("service_approval_configs")
        .update_many(doc! {}, doc! {"$set":{"approval_mode":"grant"}})
        .await
        .unwrap();
    // Legacy approvals bind AuthUser's canonical requester (the token
    // subject for this key), not the new contract's explicit key principal.
    let requester = f.auth().await.approval_requester_id();
    let mut grant = f.grant().await;
    grant.requester_id = requester.clone();
    grant.execution = None;
    grant.scope = Some("v1:http:get:read:/status".into());
    grant.expires_at = Utc::now() + chrono::Duration::hours(1);
    let mut original = bson::to_document(&grant).unwrap();
    original.remove("org_scoped"); // Pre-org-scope schema remains personal.
    f.state
        .db
        .collection::<Document>("approval_grants")
        .insert_one(&original)
        .await
        .unwrap();
    let descriptor =
        crate::services::operation_descriptor::build_http_descriptor("GET", "status", None);
    assert!(approval_policy::grant_scope_covers(
        grant.scope.as_deref(),
        &descriptor
    ));
    for candidate in [requester.as_str(), "another-requester"] {
        assert_eq!(
            approval_service::check_approval(
                &f.state.db,
                &f.owner,
                &f.catalog,
                "api_key",
                candidate,
                false,
                &descriptor
            )
            .await
            .unwrap(),
            candidate == requester
        );
    }
    for route in [
        format!("/proxy/{}/status", f.catalog),
        "/proxy/s/execution-fixture/status".into(),
    ] {
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), f.call(&route)).await;
        assert!(result.is_ok(), "unexpected approval wait on {route}");
        let (status, body) = result.unwrap();
        assert_eq!(status, StatusCode::OK, "{route}: {body}");
    }
    assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
    assert_eq!(
        f.state
            .db
            .collection::<Document>("approval_grants")
            .find_one(doc! {"_id":&grant.id})
            .await
            .unwrap()
            .unwrap(),
        original
    );
    // The same saved grant must not satisfy per-request mode.
    f.state
        .db
        .collection::<Document>("service_approval_configs")
        .update_many(doc! {}, doc! {"$set":{"approval_mode":"per_request"}})
        .await
        .unwrap();
    assert!(matches!(
        approval_service::evaluate_and_check(
            &f.state.db,
            &f.owner,
            &f.owner,
            &f.catalog,
            &descriptor,
            Some("api_key"),
            &requester,
            false,
            false
        )
        .await
        .unwrap(),
        approval_service::ApprovalOutcome::NeedsApproval(_)
    ));
    for patch in [
        doc! {"revoked":true},
        doc! {"revoked":false,"expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))},
    ] {
        f.state
            .db
            .collection::<Document>("approval_grants")
            .update_one(doc! {"_id":&grant.id}, doc! {"$set":patch})
            .await
            .unwrap();
        assert!(
            !approval_service::check_approval(
                &f.state.db,
                &f.owner,
                &f.catalog,
                "api_key",
                &requester,
                false,
                &descriptor
            )
            .await
            .unwrap()
        );
    }
    f.no_prompts().await;
    assert_eq!(count(&f, "approval_grants").await, 1);
    f.cleanup().await;
}

#[tokio::test]
async fn migration_input_rules_remain_a_ceiling_over_explicit_permission() {
    let f = Fixture::new("migration_input_ceiling").await;
    f.enroll(true).await;
    let grant = f.grant().await;
    f.store(&grant).await;
    let mut scope = grant.execution.unwrap().v1.unwrap().operations;
    scope.operations[0].inputs = Some(
        serde_json::from_value(
            json!({"query":{"range":{"required":true,"rule":{"type":"exact","value":"A1"}}}}),
        )
        .unwrap(),
    );
    f.state
        .db
        .collection::<Document>("api_keys")
        .update_one(
            doc! {"_id":&f.key},
            doc! {"$set":{
                "assistant_operation_scopes":{&f.connection:bson::to_bson(&scope).unwrap()}
            }},
        )
        .await
        .unwrap();
    let (status, body) = f.call("/proxy/s/execution-fixture/status?range=A1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _) = f.call("/proxy/s/execution-fixture/status?range=A2").await;
    assert_ne!(status, StatusCode::OK);
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    f.no_prompts().await;
    f.cleanup().await;
}

#[tokio::test]
async fn migration_seeded_same_audience_permission_covers_only_explicit_new_conversations() {
    let (mut f, first) = super::proposal_tests::assistant("migration_same_audience").await;
    let grant = seed_for_chat(
        &f.state,
        &f.auth().await,
        &f.connection,
        &f.catalog,
        "GET",
        "/status",
    )
    .await;
    let (status, body) = f.call("/proxy/s/execution-fixture/status").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let input = engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        agent_id: first.agent_id.clone(),
        conversation_id: None,
        text: "A new conversation".into(),
        model: None,
        access_mode: None,
    };
    let mut start: engine::TurnStart = (&input).into();
    start.execution_enrollment = true;
    let second = engine::begin_turn(&f.state.db, &f.owner, start, &f.state.encryption_keys)
        .await
        .unwrap();
    assert_ne!(first.id, second.id);
    f.key = second.credential_api_key_id.clone();
    f.raw = crate::services::assistant_agent_credential_service::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &second.id,
    )
    .await
    .unwrap()
    .unwrap()
    .raw_key
    .to_string();
    let auth = f.auth().await;
    let target = f.target().await;
    let effect = || Effect {
        connection: &f.connection,
        owner: &f.owner,
        target: &target,
        method: "GET",
        path: "status",
        query: None,
        body: b"",
        content_type: None,
        headers: &[],
        endpoint: None,
        floors: &[ExecutionFloor::PersonalServiceApproval],
    };
    assert!(
        prepare(
            &f.state,
            auth.execution_permission.as_ref(),
            auth.assistant_chat.as_deref(),
            &f.key,
            effect()
        )
        .await
        .unwrap()
        .is_none(),
        "old Exact contract does not become cross-conversation"
    );
    // Reader test only: slice (b) must supply the human-authored choice.
    f.state
        .db
        .collection::<Document>("approval_grants")
        .update_one(
            doc! {"_id":&grant.id},
            doc! {"$set":{"execution.v1.context.conversation":{"kind":"same_audience"}}},
        )
        .await
        .unwrap();
    let (status, body) = f.call("/proxy/s/execution-fixture/status").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
    f.no_prompts().await;
    f.cleanup().await;
}

#[tokio::test]
async fn migration_diagnostics_aggregate_candidates_and_prepared_reentry_without_inputs() {
    use tracing::instrument::WithSubscriber;
    use tracing_subscriber::prelude::*;
    let f = Fixture::new("migration_diagnostics").await;
    f.enroll(true).await;
    let captured = Capture::default();
    async {
        let target = f.target().await;
        let run = |ctx| {
            prepare(
                &f.state,
                ctx,
                None,
                &f.key,
                Effect {
                    connection: &f.connection,
                    owner: &f.owner,
                    target: &target,
                    method: "GET",
                    path: "status",
                    query: Some("secret=do-not-log-me"),
                    body: b"",
                    content_type: None,
                    headers: &[],
                    endpoint: None,
                    floors: &[ExecutionFloor::PersonalServiceApproval],
                },
            )
        };
        let gap = RequestContext::enrolled(Some(1), &f.owner, &f.key, None).unwrap();
        assert!(run(Some(&gap)).await.unwrap().is_none());
        assert!(run(Some(&gap)).await.unwrap().is_none());
        let grant = f.grant().await;
        let mut other = grant.clone();
        other.id = Uuid::new_v4().to_string();
        other
            .execution
            .as_mut()
            .unwrap()
            .v1
            .as_mut()
            .unwrap()
            .binding
            .credential_epoch += 1;
        f.store(&other).await;
        f.store(&grant).await;
        let covered = RequestContext::enrolled(Some(1), &f.owner, &f.key, None).unwrap();
        assert!(run(Some(&covered)).await.unwrap().is_some());
        assert!(run(Some(&covered)).await.unwrap().is_some());
        f.state
            .db
            .collection::<Document>("approval_grants")
            .delete_one(doc! {"_id":&other.id})
            .await
            .unwrap();
        f.state
            .db
            .collection::<Document>("approval_grants")
            .update_one(doc! {"_id":&grant.id}, doc! {"$set":{"revoked":true}})
            .await
            .unwrap();
        let denied = RequestContext::enrolled(Some(1), &f.owner, &f.key, None).unwrap();
        assert!(run(Some(&denied)).await.is_err());
        assert!(run(None).await.unwrap().is_none());
    }
    .with_subscriber(tracing_subscriber::registry().with(captured.clone()))
    .await;
    let events = captured.0.lock().unwrap().clone();
    assert_eq!(
        events.len(),
        3,
        "one aggregate event per participating occurrence"
    );
    assert_eq!(
        events
            .iter()
            .map(|v| v["cause"].as_str())
            .collect::<Vec<_>>(),
        [
            "permission_missing",
            "permission_covered",
            "permission_revoked"
        ]
    );
    assert_eq!(events[1]["covered_validity"], "persistent");
    assert_eq!(events[1]["covered_lane"], "Some(Owner)");
    assert_eq!(
        events[1]["covered_floors"],
        "Some([PersonalServiceApproval])"
    );
    for event in &events {
        assert_eq!(
            event.len(),
            if event["decision"] == "covered" {
                12
            } else {
                11
            }
        );
        assert!(!format!("{event:?}").contains("do-not-log-me"));
        assert!(!event.contains_key("path"));
        assert!(!event.contains_key("actor"));
    }
    f.no_prompts().await;
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn migration_unavailable_authority_is_operational_without_review_or_effect() {
    let f = Fixture::new("migration_dependency").await;
    f.enroll(true).await;
    let target = f.target().await;
    let mut failed_state = f.state.clone();
    let options = mongodb::options::ClientOptions::parse("mongodb://127.0.0.1:9/?directConnection=true&serverSelectionTimeoutMS=100&connectTimeoutMS=100").await.unwrap();
    failed_state.db = mongodb::Client::with_options(options)
        .unwrap()
        .database(f.state.db.name());
    let ctx = RequestContext::enrolled(Some(1), &f.owner, &f.key, None).unwrap();
    let result = prepare(
        &failed_state,
        Some(&ctx),
        None,
        &f.key,
        Effect {
            connection: &f.connection,
            owner: &f.owner,
            target: &target,
            method: "GET",
            path: "status",
            query: None,
            body: b"",
            content_type: None,
            headers: &[],
            endpoint: None,
            floors: &[ExecutionFloor::PersonalServiceApproval],
        },
    )
    .await;
    assert!(result.is_err());
    let observed = ctx.observation.reported.get().unwrap();
    assert_eq!(observed.outcome, "operational_error");
    assert_eq!(observed.cause, "enrollment_resolution_unavailable");
    f.no_prompts().await;
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn migration_expired_oauth_refreshes_then_executes_without_new_consent() {
    use crate::services::user_api_key_service::{self as keys, CreateApiKeyParams};
    use wiremock::matchers::{body_string_contains, header, method, path};
    let f = Fixture::new("migration_oauth_refresh").await;
    f.enroll(true).await;
    let tokens = MockServer::start().await;
    Mock::given(method("POST")).and(path("/token")).and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token":"refreshed-fixture-token","refresh_token":"rotated-fixture-refresh","token_type":"Bearer","expires_in":3600})))
        .expect(1).mount(&tokens).await;
    f.server.reset().await;
    Mock::given(method("GET"))
        .and(path("/status"))
        .and(header("authorization", "Bearer refreshed-fixture-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
        .expect(1)
        .mount(&f.server)
        .await;
    let provider_id = Uuid::new_v4().to_string();
    let client = f
        .state
        .encryption_keys
        .encrypt(b"fixture-client")
        .await
        .unwrap();
    let provider: crate::models::provider_config::ProviderConfig = bson::from_document(doc! {
        "_id":&provider_id,"slug":"migration-oauth","name":"Fixture","provider_type":"oauth2","is_active":true,
        "created_by":&f.owner,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
        "token_url":format!("{}/token", tokens.uri()),"client_id_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:client},
        "token_endpoint_auth_method":"none"
    }).unwrap();
    f.state
        .db
        .collection::<crate::models::provider_config::ProviderConfig>("provider_configs")
        .insert_one(&provider)
        .await
        .unwrap();
    let credential = keys::build_api_key(
        &f.state.encryption_keys,
        &f.owner,
        CreateApiKeyParams {
            label: "OAuth fixture",
            credential_type: "oauth2",
            credential: "",
            access_token: Some("expired-fixture-token"),
            refresh_token: Some("fixture-refresh"),
            token_scopes: None,
            expires_at: Some(Utc::now() - chrono::Duration::minutes(1)),
            provider_config_id: Some(&provider_id),
            connection_id: Some(&f.connection),
            oauth_client_id: None,
            oauth_client_secret: None,
            status: "active",
            source: None,
            source_id: None,
        },
    )
    .await
    .unwrap();
    f.state
        .db
        .collection::<crate::models::user_api_key::UserApiKey>("user_api_keys")
        .insert_one(&credential)
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id":&f.connection},
            doc! {"$set":{"api_key_id":&credential.id,"auth_method":"bearer","auth_key_name":"Authorization"}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! {"_id":&f.catalog},
            doc! {"$set":{"auth_method":"bearer","requires_user_credential":true}},
        )
        .await
        .unwrap();
    let grant = f.grant().await;
    f.store(&grant).await;
    let (status, body) = f.call("/proxy/s/execution-fixture/status").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let refreshed = f
        .state
        .db
        .collection::<crate::models::user_api_key::UserApiKey>("user_api_keys")
        .find_one(doc! {"_id":&credential.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(refreshed.credential_epoch, credential.credential_epoch);
    assert!(refreshed.expires_at.unwrap() > Utc::now());
    assert_eq!(tokens.received_requests().await.unwrap().len(), 1);
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    f.no_prompts().await;
    f.cleanup().await;
}

#[tokio::test]
async fn migration_delegated_task_uses_child_permission_and_keeps_report_target() {
    use crate::services::assistant_team_service as team;
    let (mut f, parent) = super::proposal_tests::assistant("migration_delegated_effect").await;
    let parent_auth = f.auth().await;
    seed_for_chat(
        &f.state,
        &parent_auth,
        &f.connection,
        &f.catalog,
        "GET",
        "/status",
    )
    .await;
    let (agent, home) = team::create_specialist(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        team::CreateRequest {
            machines: None,
            logins: None,
            name: "migration-worker".into(),
            description: "Test specialist".into(),
            display_name: None,
            persona: None,
            targets: team::GrantTargets {
                service_ids: vec![f.connection.clone()],
                ..Default::default()
            },
            account_read: false,
            specialty: None,
            created_by: "user",
        },
    )
    .await
    .unwrap()
    .unwrap();
    let input = engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        agent_id: Some(agent.id.clone()),
        conversation_id: Some(home.id.clone()),
        text: "Check the admitted task's status".into(),
        model: None,
        access_mode: None,
    };
    let mut start: engine::TurnStart = (&input).into();
    start.origin = crate::models::assistant_conversation::TurnOrigin::Orchestrator;
    start.execution_parent = tasks::parent(parent_auth.assistant_chat.as_deref().unwrap());
    start.report_to = Some(parent.id.clone());
    let child = engine::begin_turn(&f.state.db, &f.owner, start, &f.state.encryption_keys)
        .await
        .unwrap();
    let task = child
        .active_turn
        .as_ref()
        .unwrap()
        .execution_task
        .as_ref()
        .unwrap();
    assert_eq!(
        task.task,
        parent
            .active_turn
            .as_ref()
            .unwrap()
            .execution_task
            .as_ref()
            .unwrap()
            .task
    );
    f.key = child.credential_api_key_id.clone();
    f.raw = crate::services::assistant_agent_credential_service::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &child.id,
    )
    .await
    .unwrap()
    .unwrap()
    .raw_key
    .to_string();
    let auth = f.auth().await;
    let target = f.target().await;
    let result = prepare(
        &f.state,
        auth.execution_permission.as_ref(),
        auth.assistant_chat.as_deref(),
        &f.key,
        Effect {
            connection: &f.connection,
            owner: &f.owner,
            target: &target,
            method: "GET",
            path: "status",
            query: None,
            body: b"",
            content_type: None,
            headers: &[],
            endpoint: None,
            floors: &[ExecutionFloor::PersonalServiceApproval],
        },
    )
    .await
    .unwrap();
    assert!(
        result.is_none(),
        "task lineage never transfers the delegator's consent"
    );
    let grant = seed_for_chat(&f.state, &auth, &f.connection, &f.catalog, "GET", "/status").await;
    f.state.db.collection::<Document>("approval_grants").update_one(doc! {"_id":grant.id},doc! {"$set":{
        "execution.v1.validity":bson::to_bson(&Validity::Task{task:task.task.clone(),hard_deadline:task.hard_deadline}).unwrap(),
        "expires_at":bson::DateTime::from_chrono(task.hard_deadline)
    }}).await.unwrap();
    let (status, body) = f.call("/proxy/s/execution-fixture/status").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    engine::finish_turn(
        &f.state.db,
        &child,
        &f.key,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "The status is ready".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    assert!(f.state.db.collection::<Document>("assistant_messages").find_one(doc! {"conversation_id":&child.id,"role":"assistant","report_to":&parent.id,"status":"completed"}).await.unwrap().is_some());
    tasks::stop(&f.state.db, &parent).await.unwrap();
    assert!(tasks::check(&f.state.db, task).await.is_err());
    f.no_prompts().await;
    f.cleanup().await;
}

#[test]
fn migration_cancelled_admission_emits_one_safe_operational_event() {
    use futures::FutureExt;
    let captured = Capture::default();
    tracing::dispatcher::with_default(&captured.subscriber(), || {
        let context =
            RequestContext::enrolled(Some(1), "fixture-owner", "fixture-key", None).unwrap();
        assert!(
            diagnostics::observe(Some(&context), futures::future::pending::<AppResult<()>>())
                .now_or_never()
                .is_none()
        );
        drop(context);
    });
    let events = captured.0.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["decision"], "operational_error");
    assert_eq!(events[0]["cause"], "admission_abandoned");
    assert!(!format!("{events:?}").contains("fixture-key"));
    assert!(!format!("{events:?}").contains("fixture-owner"));
}

#[test]
fn migration_legacy_diagnostic_levels_are_debug_and_aggregate_once() {
    let captured = Capture::default();
    let causes = [
        "legacy_flag_off",
        "legacy_no_eligible_floor",
        "legacy_unsupported_source",
        "legacy_unsupported_transport",
    ];
    tracing::dispatcher::with_default(&captured.subscriber(), || {
        for cause in causes {
            let context = RequestContext::enrolled(Some(1), "owner", "key", None).unwrap();
            context.observation.start();
            if cause != "legacy_no_eligible_floor" {
                context
                    .observation
                    .decide(diagnostics::Decision::new("legacy", cause));
            }
            context.observation.finish(&context, &Ok::<_, AppError>(()));
            context.observation.finish(&context, &Ok::<_, AppError>(()));
        }
    });
    let events = captured.0.lock().unwrap();
    assert_eq!(events.len(), causes.len());
    for (event, cause) in events.iter().zip(causes) {
        assert_eq!(event["decision"], "legacy");
        assert_eq!(event["cause"], cause);
    }
}
