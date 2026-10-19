#![cfg(test)]

use super::{
    platform_onboarding_test_support::{self as support, Fixture},
    platform_provisioning_service as provisioning, platform_runtime_service as runtime,
    platform_setup_service as setups, telegram_new_tests as telegram,
};
use crate::models::{
    platform_installation::{self, PlatformInstallation},
    platform_setup::{PlatformSetup, SetupState, SetupTemplate},
    telegram_bot_request::{self, TelegramBotRequest, TelegramRequestStatus as Status},
};
use mongodb::bson::{Document, doc};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Notify;

async fn native(f: &Fixture) -> TelegramBotRequest {
    f.state
        .db
        .collection::<TelegramBotRequest>(telegram_bot_request::COLLECTION_NAME)
        .find_one(doc! {"_id": f.setup.resources.creation_request_id.as_ref().unwrap()})
        .await
        .unwrap()
        .unwrap()
}

async fn installation(f: &Fixture) -> PlatformInstallation {
    f.state
        .db
        .collection::<PlatformInstallation>(platform_installation::COLLECTION_NAME)
        .find_one(doc! {"setup_id": &f.setup.id})
        .await
        .unwrap()
        .unwrap()
}

async fn reapprove(f: &Fixture) -> PlatformSetup {
    let live = setups::get_owned(&f.state.db, &f.owner, &f.setup.id)
        .await
        .unwrap();
    let paused = setups::pause_or_cancel(&f.state.db, &f.owner, &live.id, live.revision, false)
        .await
        .unwrap();
    let review = setups::request_review(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &paused.id,
        paused.revision,
    )
    .await
    .unwrap();
    let facts = setups::decrypt_facts(&f.state.encryption_keys, &review)
        .await
        .unwrap();
    let digest = setups::approval_digest(&review, &facts).unwrap();
    provisioning::approve_owned(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &review.id,
        review.revision,
        &digest,
    )
    .await
    .unwrap()
}

async fn approve_observed(f: &Fixture) {
    Box::pin(f.observe()).await;
    let (candidate, request, digest) =
        provisioning::child_candidate(&f.state.db, &f.state.encryption_keys, &f.owner, &f.setup.id)
            .await
            .unwrap();
    provisioning::approve_child_owned(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.setup.id,
        candidate.revision - 1,
        request.revision,
        900,
        request.manager_revision.unwrap(),
        &digest,
    )
    .await
    .unwrap();
    let job = f.claim().await;
    Box::pin(provisioning::execute_step(&f.state, &job))
        .await
        .unwrap();
}

async fn assert_single_resources(f: &Fixture, original: &PlatformInstallation) {
    let live = installation(f).await;
    assert_eq!(live.id, original.id);
    assert_eq!(live.channel_bot_id, original.channel_bot_id);
    assert_eq!(live.agent_id, original.agent_id);
    assert_eq!(live.telegram_bot_id.as_deref(), Some("900"));
    for (collection, filter) in [
        (
            platform_installation::COLLECTION_NAME,
            doc! {"setup_id": &f.setup.id},
        ),
        (
            crate::models::assistant_agent::COLLECTION_NAME,
            doc! {"platform_setup_id": &f.setup.id, "user_id": &f.owner},
        ),
        (
            crate::models::channel_bot::COLLECTION_NAME,
            doc! {"platform_bot_id": "900", "user_id": &f.owner},
        ),
    ] {
        assert_eq!(
            f.state
                .db
                .collection::<Document>(collection)
                .count_documents(filter)
                .await
                .unwrap(),
            1
        );
    }
}

async fn activate_ready(f: &Fixture, update_id: i64) -> PlatformSetup {
    let installed = installation(f).await;
    let customer = runtime::capture(&f.state, &installed.channel_bot_id, 800, update_id, true)
        .await
        .unwrap();
    let (test, code) = runtime::begin_test(&f.state, &customer).await.unwrap();
    let base = f.server.uri();
    let service = telegram::service(&f.state, &base);
    let reply = service.api.call("900:child-test-secret", "sendMessage", json!({"chat_id": 800, "text": format!("Independent delivery test: {}", code.as_str())})).await.unwrap();
    let message_id = reply["message_id"].as_i64().unwrap();
    runtime::observed_test_send(&f.state, &customer, &test, message_id)
        .await
        .unwrap();
    assert!(
        runtime::confirm_test(
            &f.state,
            &installed.channel_bot_id,
            800,
            message_id,
            code.as_str(),
            update_id + 1
        )
        .await
        .unwrap()
    );
    let ready = setups::get_owned(&f.state.db, &f.owner, &f.setup.id)
        .await
        .unwrap();
    Box::pin(runtime::activate_owned(
        &f.state,
        &f.owner,
        &ready.id,
        ready.revision,
    ))
    .await
    .unwrap()
}

#[tokio::test]
async fn platform_onboarding_connected_and_active_reapproval_preserve_identity_and_require_new_delivery()
 {
    let f = Box::pin(support::funded_fixture()).await;
    approve_observed(&f).await;
    let original = installation(&f).await;
    telegram::provider_connection(&f.server, &original.channel_bot_id, 200).await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path(
            "/bot900:child-test-secret/sendMessage",
        ))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(json!({"ok": true, "result": {"message_id": 333}})),
        )
        .mount(&f.server)
        .await;
    let base = f.server.uri();
    let service = telegram::service(&f.state, &base);
    Box::pin(service.complete_pending_creations())
        .await
        .unwrap();
    let old_connected = native(&f).await;
    assert_eq!(old_connected.status, Status::Connected);
    reapprove(&f).await;
    assert!(
        Box::pin(service.connect_claimed(&old_connected, 900))
            .await
            .is_err()
    );
    let connected_job = f.claim().await;
    Box::pin(provisioning::execute_step(&f.state, &connected_job))
        .await
        .unwrap();
    let ready = setups::get_owned(&f.state.db, &f.owner, &f.setup.id)
        .await
        .unwrap();
    assert_eq!(ready.state, SetupState::ReadyForTest);
    assert!(!ready.delivery_tested);
    assert!(
        Box::pin(runtime::activate_owned(
            &f.state,
            &f.owner,
            &ready.id,
            ready.revision
        ))
        .await
        .is_err()
    );
    let active = Box::pin(activate_ready(&f, 400)).await;
    assert_eq!(active.state, SetupState::Active);
    let before_replay = installation(&f).await;
    let claim = f.state.db.collection::<crate::models::platform_installation::SetupStepJob>(platform_installation::STEPS)
        .find_one(doc! {"_id": &connected_job.id, "phase": "running", "generation": connected_job.generation,
            "claim_id": &connected_job.claim_id, "$expr": {"$gt": ["$retry_at", "$$NOW"]}}).await.unwrap().unwrap();
    assert_eq!(claim.authority, connected_job.authority);
    Box::pin(provisioning::execute_step(&f.state, &connected_job))
        .await
        .unwrap();
    let after_replay = installation(&f).await;
    let active_after_replay = setups::get_owned(&f.state.db, &f.owner, &f.setup.id)
        .await
        .unwrap();
    assert_eq!(active_after_replay.state, SetupState::Active);
    assert_eq!(active_after_replay.revision, active.revision);
    assert_eq!(
        active_after_replay.authority_generation,
        active.authority_generation
    );
    assert_eq!(after_replay.state, "active");
    assert_eq!(after_replay.generation, before_replay.generation);
    assert_eq!(after_replay.authority, before_replay.authority);
    let current = native(&f).await;
    let before_retry = installation(&f).await;
    Box::pin(service.connect(&f.owner, &current.id, 900, current.revision))
        .await
        .unwrap();
    let after_retry = installation(&f).await;
    assert_eq!(
        after_retry.state, "active",
        "idempotent native retry must preserve active public authority"
    );
    assert_eq!(after_retry.generation, before_retry.generation);
    runtime::capture(&f.state, &original.channel_bot_id, 800, 402, false)
        .await
        .unwrap();
    reapprove(&f).await;
    assert!(
        runtime::capture(&f.state, &original.channel_bot_id, 800, 403, false)
            .await
            .is_err()
    );
    assert!(
        Box::pin(provisioning::execute_step(&f.state, &connected_job))
            .await
            .is_err()
    );
    let fresh = f.claim().await;
    Box::pin(provisioning::execute_step(&f.state, &fresh))
        .await
        .unwrap();
    let ready = setups::get_owned(&f.state.db, &f.owner, &f.setup.id)
        .await
        .unwrap();
    assert_eq!(ready.state, SetupState::ReadyForTest);
    assert!(!ready.delivery_tested);
    assert!(
        Box::pin(runtime::activate_owned(
            &f.state,
            &f.owner,
            &ready.id,
            ready.revision
        ))
        .await
        .is_err()
    );
    assert_eq!(
        Box::pin(activate_ready(&f, 410)).await.state,
        SetupState::Active
    );
    assert_single_resources(&f, &original).await;
    assert_eq!(
        f.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path().ends_with("/getManagedBotToken"))
            .count(),
        1
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn platform_onboarding_specialist_preflight_retries_owner_write_conflict() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    let f = Box::pin(support::distinct_fixture()).await;
    approve_observed(&f).await;
    let original = installation(&f).await;
    telegram::provider_connection(&f.server, &original.channel_bot_id, 200).await;
    let base = f.server.uri();
    let service = telegram::service(&f.state, &base);
    Box::pin(service.complete_pending_creations())
        .await
        .unwrap();
    assert_eq!(native(&f).await.status, Status::Connected);
    let agents = f
        .state
        .db
        .collection::<crate::models::assistant_agent::AssistantAgent>(
            crate::models::assistant_agent::COLLECTION_NAME,
        );
    let original_agent = agents
        .find_one(doc! {"_id": &original.agent_id})
        .await
        .unwrap()
        .unwrap();
    reapprove(&f).await;
    let fresh = f.claim().await;

    let conflict = Arc::new(Notify::new());
    let conflicts = Arc::new(AtomicUsize::new(0));
    let signal = conflict.clone();
    let observed = conflicts.clone();
    let handler = EventHandler::callback(move |event| {
        if let CommandEvent::Failed(event) = event
            && event.command_name == "update"
            && event.failure.contains_label("TransientTransactionError")
            && matches!(
                event.failure.kind.as_ref(),
                mongodb::error::ErrorKind::Command(error) if error.code == 112
            )
        {
            observed.fetch_add(1, Ordering::SeqCst);
            signal.notify_one();
        }
    });
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(handler);
    let mut state = f.state.clone();
    state.db = mongodb::Client::with_options(options)
        .unwrap()
        .database(f.state.db.name());

    // Hold the real owner write until the preflight encounters contention.
    // Release by command observation, not a sleep or a whole-operation retry.
    let mut blocker = f.state.db.client().start_session().await.unwrap();
    blocker.start_transaction().await.unwrap();
    setups::fence_owner(&f.state.db, &f.owner, &mut blocker)
        .await
        .unwrap();
    let (result, ()) = tokio::join!(
        async {
            tokio::time::timeout(
                Duration::from_secs(30),
                Box::pin(provisioning::execute_step(&state, &fresh)),
            )
            .await
            .expect("provisioning must finish after the competing writer rolls back")
        },
        async {
            let observed = tokio::time::timeout(Duration::from_secs(10), conflict.notified()).await;
            blocker.abort_transaction().await.unwrap();
            observed.expect("the real preflight must encounter a transient transaction error");
        },
    );
    result.expect("specialist authority preflight must retry transient owner write conflicts");
    assert!(conflicts.load(Ordering::SeqCst) > 0);
    assert_single_resources(&f, &original).await;
    let ready = setups::get_owned(&f.state.db, &f.owner, &f.setup.id)
        .await
        .unwrap();
    assert_eq!(ready.state, SetupState::ReadyForTest);
    assert!(!ready.delivery_tested);
    let current_agent = agents
        .find_one(doc! {"_id": &original.agent_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        current_agent.home_conversation_id,
        original_agent.home_conversation_id
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .count_documents(doc! {"agent_id": &original.agent_id, "user_id": &f.owner})
            .await
            .unwrap(),
        1
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn platform_onboarding_registered_reapproval_resumes_saved_native_worker_without_new_token() {
    let f = Box::pin(support::distinct_fixture()).await;
    approve_observed(&f).await;
    let original = installation(&f).await;
    telegram::provider_connection(&f.server, &original.channel_bot_id, 503).await;
    let base = f.server.uri();
    let service = telegram::service(&f.state, &base);
    Box::pin(service.complete_pending_creations())
        .await
        .unwrap();
    let saved = super::channel_bot_service::get_bot(&f.state.db, &original.channel_bot_id)
        .await
        .unwrap();
    assert_eq!(saved.status, "pending");
    assert_eq!(native(&f).await.status, Status::Provisioning);
    let stale_worker = native(&f).await;
    reapprove(&f).await;
    assert!(!native(&f).await.auto_connect);
    assert!(
        Box::pin(service.connect_claimed(&stale_worker, 900))
            .await
            .is_err()
    );
    let job = f.claim().await;
    Box::pin(provisioning::execute_step(&f.state, &job))
        .await
        .unwrap();
    let resumed = native(&f).await;
    assert!(
        resumed.auto_connect,
        "saved provisioning must be eligible for the real native worker"
    );
    assert!(resumed.next_connection_attempt_at.is_none());
    f.server.reset().await;
    telegram::provider_connection(&f.server, &original.channel_bot_id, 200).await;
    Box::pin(service.complete_pending_creations())
        .await
        .unwrap();
    let connected = super::channel_bot_service::get_bot(&f.state.db, &original.channel_bot_id)
        .await
        .unwrap();
    assert_eq!(connected.status, "active");
    assert_eq!(connected.bot_token_encrypted, saved.bot_token_encrypted);
    assert_eq!(native(&f).await.status, Status::Connected);
    assert_eq!(
        setups::get_owned(&f.state.db, &f.owner, &f.setup.id)
            .await
            .unwrap()
            .state,
        SetupState::ReadyForTest
    );
    assert!(
        !f.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path().ends_with("/getManagedBotToken"))
    );
    assert_single_resources(&f, &original).await;
    f.state.db.drop().await.unwrap();
}

#[derive(Clone)]
struct RegistrationGate {
    upstream: String,
    entered: Arc<Notify>,
    resume: Arc<Notify>,
    http: reqwest::Client,
}

async fn gated_telegram(
    axum::extract::State(gate): axum::extract::State<RegistrationGate>,
    request: axum::extract::Request,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let path = request.uri().path().to_owned();
    if path == "/bot900:child-test-secret/getMe" {
        gate.entered.notify_one();
        gate.resume.notified().await;
    }
    let body = axum::body::to_bytes(request.into_body(), 16 * 1024)
        .await
        .unwrap();
    let upstream = gate
        .http
        .post(format!("{}{path}", gate.upstream))
        .body(body)
        .send()
        .await
        .unwrap();
    let status = upstream.status();
    (status, upstream.bytes().await.unwrap()).into_response()
}

#[tokio::test]
async fn platform_onboarding_independent_native_claim_unlink_before_registration_commits_no_child()
{
    let f = Box::pin(support::distinct_fixture()).await;
    approve_observed(&f).await;
    let original = installation(&f).await;
    let before = native(&f).await;
    assert_eq!(before.status, Status::Ready);
    telegram::provider_connection(&f.server, &original.channel_bot_id, 200).await;
    let gate = RegistrationGate {
        upstream: f.server.uri(),
        entered: Arc::new(Notify::new()),
        resume: Arc::new(Notify::new()),
        http: f.state.http_client.clone(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new()
        .fallback(gated_telegram)
        .with_state(gate.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let service = telegram::service(&f.state, &base);
    let revoke = async {
        tokio::time::timeout(std::time::Duration::from_secs(10), gate.entered.notified())
            .await
            .unwrap();
        let claimed = native(&f).await;
        assert_eq!(claimed.connection_attempts, before.connection_attempts + 1);
        assert!(claimed.connection_generation > before.connection_generation);
        assert_eq!(
            f.state
                .db
                .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
                .count_documents(doc! {"_id": &original.channel_bot_id})
                .await
                .unwrap(),
            0
        );
        super::platform_binding_service::unlink_sender(&f.state.db, &f.profile, "700", "700")
            .await
            .unwrap();
        gate.resume.notify_one();
    };
    let (worker, ()) = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        tokio::join!(Box::pin(service.complete_pending_creations()), revoke)
    })
    .await
    .unwrap();
    worker.unwrap();
    server.abort();
    assert_eq!(
        f.state
            .db
            .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .count_documents(doc! {"_id": &original.channel_bot_id})
            .await
            .unwrap(),
        0
    );
    assert!(
        provisioning::admit_request(&f.state.db, &native(&f).await, true)
            .await
            .is_err()
    );
    let event = f
        .state
        .db
        .collection::<telegram_bot_request::ManagedBotEvents>(telegram_bot_request::MANAGED_BOTS)
        .find_one(doc! {"telegram_bot_id": 900_i64})
        .await
        .unwrap()
        .unwrap();
    assert!(
        !event.retired,
        "failed registration must not consume the Telegram creation"
    );
    assert!(
        !f.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path().ends_with("/setWebhook"))
    );
    f.state.db.drop().await.unwrap();
}

async fn second_setup(f: &Fixture) -> PlatformSetup {
    let facts = setups::decrypt_facts(&f.state.encryption_keys, &f.setup)
        .await
        .unwrap();
    let row = setups::create_owned(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &uuid::Uuid::new_v4().to_string(),
        SetupTemplate::RestaurantStarter,
        &facts,
    )
    .await
    .unwrap();
    let row = setups::attach_binding(
        &f.state.db,
        &f.owner,
        &row.id,
        row.revision,
        &f.binding.id,
        f.binding.generation,
    )
    .await
    .unwrap();
    let row = setups::publish_reviewed(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &row.id,
        row.revision,
    )
    .await
    .unwrap();
    let row = setups::request_review(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &row.id,
        row.revision,
    )
    .await
    .unwrap();
    let digest = setups::approval_digest(&row, &facts).unwrap();
    provisioning::approve_owned(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &row.id,
        row.revision,
        &digest,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn platform_onboarding_competing_creation_and_late_form_require_exact_child_review() {
    let f = Box::pin(support::distinct_fixture()).await;
    let first = Box::pin(f.native_start()).await;
    let second = second_setup(&f).await;
    let job = f.claim().await;
    assert_eq!(job.setup_id, second.id);
    let base = f.server.uri();
    let service = telegram::service(&f.state, &base);
    assert!(service.begin_approved(&job, "Second cafe").await.is_err());
    assert!(
        f.state
            .db
            .collection::<TelegramBotRequest>(telegram_bot_request::COLLECTION_NAME)
            .find_one(doc! {"_id": second.resources.creation_request_id.as_ref().unwrap()})
            .await
            .unwrap()
            .is_none()
    );
    service.cancel(&f.owner, &first.id).await.unwrap();
    let (_, url) = service.begin_approved(&job, "Second cafe").await.unwrap();
    let challenge = reqwest::Url::parse(&url)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "start")
        .unwrap()
        .1
        .to_string();
    let headers = telegram::headers(&f.state).await;
    let mut start = telegram::message(json!({"text": format!("/start {challenge}")}));
    start["update_id"] = json!(100);
    service
        .webhook(&headers, &serde_json::to_vec(&start).unwrap())
        .await
        .unwrap();
    let child = json!({"id": 900, "is_bot": true, "username": "LateFirstFormBot"});
    service.webhook(&headers, &serde_json::to_vec(&json!({"update_id": 101, "managed_bot": {"user": {"id": 700, "is_bot": false}, "bot": child}})).unwrap()).await.unwrap();
    let mut late = telegram::message(json!({"managed_bot_created": {"bot": child}}));
    late["update_id"] = json!(102);
    service
        .webhook(&headers, &serde_json::to_vec(&late).unwrap())
        .await
        .unwrap();
    service
        .webhook(&headers, &serde_json::to_vec(&late).unwrap())
        .await
        .unwrap();
    let first_live = native(&f).await;
    assert_eq!(first_live.status, Status::Cancelled);
    assert!(first_live.telegram_bot_id.is_none());
    let (candidate, request, digest) =
        provisioning::child_candidate(&f.state.db, &f.state.encryption_keys, &f.owner, &second.id)
            .await
            .unwrap();
    assert_eq!(
        candidate.resources.observed_telegram_bot_id.as_deref(),
        Some("900")
    );
    assert!(!request.auto_connect);
    assert!(
        provisioning::approve_child_owned(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            &second.id,
            candidate.revision - 1,
            request.revision,
            901,
            request.manager_revision.unwrap(),
            &digest
        )
        .await
        .is_err()
    );
    Box::pin(service.complete_pending_creations())
        .await
        .unwrap();
    assert_eq!(
        f.state
            .db
            .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .count_documents(doc! {"platform_bot_id": "900"})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
            .count_documents(doc! {"platform_setup_id": {"$in": [&f.setup.id, &second.id]}})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        setups::get_owned(&f.state.db, &f.owner, &second.id)
            .await
            .unwrap()
            .state,
        SetupState::AwaitingChildReview
    );
    f.state.db.drop().await.unwrap();
}
