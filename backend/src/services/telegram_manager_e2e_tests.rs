use super::*;
use crate::services::channel_platform::PlatformAdapter;
use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::State,
    http::StatusCode,
    middleware,
    routing::{get, post},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

tokio::task_local! {
    // Correlate only this fixture's HTTP webhook with synchronous driver events.
    static WEBHOOK_UPDATE: i64;
}

#[derive(Clone, Debug)]
struct TransactionStatement {
    request_id: i32,
    update_id: i64,
    transaction: i64,
    session: usize,
    command: &'static str,
    collection: &'static str,
    statement: &'static str,
    code: Option<i32>,
    transient: bool,
    unknown_commit: bool,
    succeeded: bool,
}

#[derive(Default)]
struct TransactionTrace {
    pending: HashMap<i32, TransactionStatement>,
    outcomes: Vec<TransactionStatement>,
    // Session BSON stays private; diagnostics use only a fixture-local ordinal.
    sessions: Vec<bson::Document>,
    overflowed: bool,
}

fn transaction_observer(
    trace: Arc<Mutex<TransactionTrace>>,
) -> mongodb::event::EventHandler<mongodb::event::command::CommandEvent> {
    use mongodb::event::{EventHandler, command::CommandEvent};
    EventHandler::callback(move |event| {
        let mut trace = trace.lock().unwrap();
        match event {
            CommandEvent::Started(event)
                if matches!(event.command_name.as_str(), "update" | "commitTransaction") =>
            {
                let Ok(update_id) = WEBHOOK_UPDATE.try_with(|id| *id) else {
                    return;
                };
                if update_id != 15 || event.command.get_bool("autocommit") != Ok(false) {
                    return;
                }
                let Ok(transaction) = event.command.get_i64("txnNumber") else {
                    return;
                };
                let Ok(session_id) = event.command.get_document("lsid") else {
                    trace.overflowed = true;
                    return;
                };
                let session = match trace.sessions.iter().position(|id| id == session_id) {
                    Some(index) => index,
                    None if trace.sessions.len() < 8 => {
                        trace.sessions.push(session_id.clone());
                        trace.sessions.len() - 1
                    }
                    None => {
                        trace.overflowed = true;
                        return;
                    }
                };
                let commit = event.command_name == "commitTransaction";
                let collection = match event.command.get_str("update").unwrap_or_default() {
                    crate::models::user::COLLECTION_NAME => "users",
                    crate::models::org_membership::COLLECTION_NAME => "org_memberships",
                    CREDENTIALS => "platform_credentials",
                    REQUESTS => "telegram_bot_requests",
                    _ if commit => "transaction",
                    _ => "other",
                };
                let update = event
                    .command
                    .get_array("updates")
                    .ok()
                    .and_then(|a| a.first())
                    .and_then(bson::Bson::as_document);
                let query = update.and_then(|u| u.get_document("q").ok());
                let increments = update
                    .and_then(|u| u.get_document("u").ok())
                    .and_then(|u| u.get_document("$inc").ok());
                let statement = if commit {
                    "commit"
                } else if increments
                    .is_some_and(|u| u.contains_key("membership_authority_revision"))
                {
                    if query.and_then(|q| q.get_str("user_type").ok()) == Some("org") {
                        "owner_org_fence"
                    } else {
                        "actor_person_fence"
                    }
                } else if increments.is_some_and(|u| u.contains_key("telegram_creation_fence")) {
                    "telegram_creation_fence"
                } else if collection == "telegram_bot_requests" {
                    "request_mutation"
                } else {
                    "other"
                };
                if trace.pending.len() >= 32 {
                    trace.overflowed = true;
                    return;
                }
                trace.pending.insert(
                    event.request_id,
                    TransactionStatement {
                        request_id: event.request_id,
                        update_id,
                        transaction,
                        session,
                        command: if commit {
                            "commitTransaction"
                        } else {
                            "update"
                        },
                        collection,
                        statement,
                        code: None,
                        transient: false,
                        unknown_commit: false,
                        succeeded: false,
                    },
                );
            }
            CommandEvent::Succeeded(event) => {
                if let Some(mut statement) = trace.pending.remove(&event.request_id) {
                    // Mongo can report a write error in an otherwise successful
                    // command. Record exact codes/labels, never infer an abort
                    // from a mere successful reply or retain reply/error prose.
                    let write_errors = event.reply.get_array("writeErrors").ok();
                    let write_concern = event.reply.get_document("writeConcernError").ok();
                    statement.succeeded = (!event.reply.contains_key("writeErrors")
                        || write_errors.is_some_and(Vec::is_empty))
                        && !event.reply.contains_key("writeConcernError");
                    statement.code = write_errors
                        .filter(|errors| errors.len() == 1 && write_concern.is_none())
                        .and_then(|errors| errors[0].as_document())
                        .or(write_concern)
                        .and_then(|error| error.get_i32("code").ok());
                    let labels = event.reply.get_array("errorLabels").ok();
                    statement.transient = labels.is_some_and(|labels| {
                        labels
                            .iter()
                            .any(|label| label.as_str() == Some("TransientTransactionError"))
                    });
                    statement.unknown_commit = labels.is_some_and(|labels| {
                        labels
                            .iter()
                            .any(|label| label.as_str() == Some("UnknownTransactionCommitResult"))
                    });
                    if trace.outcomes.len() >= 32 {
                        trace.overflowed = true;
                    } else {
                        trace.outcomes.push(statement);
                    }
                }
            }
            CommandEvent::Failed(event) => {
                if let Some(mut statement) = trace.pending.remove(&event.request_id) {
                    use mongodb::error::{ErrorKind, WriteFailure};
                    statement.code = match event.failure.kind.as_ref() {
                        ErrorKind::Command(error) => Some(error.code),
                        ErrorKind::Write(WriteFailure::WriteError(error)) => Some(error.code),
                        ErrorKind::Write(WriteFailure::WriteConcernError(error)) => {
                            Some(error.code)
                        }
                        _ => None,
                    };
                    statement.transient = event.failure.contains_label("TransientTransactionError");
                    statement.unknown_commit = event
                        .failure
                        .contains_label("UnknownTransactionCommitResult");
                    if trace.outcomes.len() >= 32 {
                        trace.overflowed = true;
                    } else {
                        trace.outcomes.push(statement);
                    }
                }
            }
            _ => {}
        }
    })
}

#[derive(Clone)]
struct HttpFixture {
    telegram_url: String,
}

async fn create_http(
    State(state): State<crate::AppState>,
    Extension(fixture): Extension<HttpFixture>,
    auth: crate::mw::auth::AuthUser,
    tele: crate::telemetry::TelemetryContext,
    Json(body): Json<crate::handlers::channel_bots::CreateChannelBotRequest>,
) -> crate::errors::AppResult<(
    StatusCode,
    Json<crate::handlers::channel_bots::CreateChannelBotResponse>,
)> {
    let adapter = crate::services::channel_adapters::telegram::TelegramAdapter::media_test_adapter(
        &fixture.telegram_url,
    );
    crate::handlers::channel_bots::create_bot_with_adapter(
        &state,
        auth,
        tele,
        body,
        &adapter,
        &TelegramApi {
            http: &state.http_client,
            base_url: &fixture.telegram_url,
        },
    )
    .await
}

async fn manager_http(
    State(state): State<crate::AppState>,
    Extension(fixture): Extension<HttpFixture>,
    headers: HeaderMap,
    body: Bytes,
) -> crate::errors::AppResult<StatusCode> {
    let update = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|value| value["update_id"].as_i64())
        .unwrap_or(-1);
    WEBHOOK_UPDATE
        .scope(
            update,
            crate::handlers::telegram_new::webhook_with_service(
                &state,
                &service(&state, &fixture.telegram_url),
                &headers,
                &body,
            ),
        )
        .await
}

async fn reply_http(
    State(state): State<crate::AppState>,
    Extension(fixture): Extension<HttpFixture>,
    headers: HeaderMap,
    Json(body): Json<crate::handlers::channel_relay::AsyncReplyRequest>,
) -> crate::errors::AppResult<Json<crate::handlers::channel_relay::AsyncReplyResponse>> {
    let adapter = crate::services::channel_adapters::telegram::TelegramAdapter::media_test_adapter(
        &fixture.telegram_url,
    );
    crate::handlers::channel_relay::async_reply_with_test_adapter(&state, &headers, body, &adapter)
        .await
}

async fn start_http(
    state: &crate::AppState,
    server: &MockServer,
) -> (String, tokio::task::JoinHandle<()>) {
    use crate::handlers::{channel_bots, channel_conversations, telegram_new};
    use crate::mw::auth::{
        reject_api_key_tokens, reject_delegated_tokens, reject_relay_tokens,
        reject_service_account_tokens,
    };

    // Match the human-only route boundary; only Telegram's HTTP destination is injected.
    let management = Router::new()
        .route(
            "/api/v1/channel-bots",
            get(channel_bots::list_bots).post(create_http),
        )
        .route(
            "/api/v1/channel-bots/{id}",
            get(channel_bots::get_bot).delete(channel_bots::delete_bot),
        )
        .route(
            "/api/v1/channel-bots/telegram-new",
            post(telegram_new::begin),
        )
        .route(
            "/api/v1/channel-bots/telegram-new/requests/{id}",
            get(telegram_new::get),
        )
        .route(
            "/api/v1/channel-bots/telegram-new/requests/{id}/creation-link",
            post(telegram_new::creation_link),
        )
        .route(
            "/api/v1/channel-conversations",
            get(channel_conversations::list_conversations)
                .post(channel_conversations::create_conversation),
        )
        .layer(middleware::from_fn(reject_delegated_tokens))
        .layer(middleware::from_fn(reject_api_key_tokens))
        .layer(middleware::from_fn(reject_service_account_tokens))
        .layer(middleware::from_fn(reject_relay_tokens));
    let app = Router::new()
        .merge(management)
        .route(
            "/api/v1/webhooks/channel/telegram-new/manager",
            post(manager_http),
        )
        .route("/api/v1/channel-relay/reply", post(reply_http))
        .layer(Extension(HttpFixture {
            telegram_url: server.uri(),
        }))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, task)
}

fn access_token(state: &crate::AppState, actor: &str) -> String {
    crate::crypto::jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &uuid::Uuid::parse_str(actor).unwrap(),
        "read write",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap()
}

async fn response_json(response: reqwest::Response, expected: StatusCode) -> Value {
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status, expected, "{body}");
    serde_json::from_str(&body).unwrap()
}

async fn add_agent(
    state: &crate::AppState,
    actor: &str,
    server: &MockServer,
    callback: &str,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    state.db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME).insert_one(doc! {
        "_id": &id, "user_id": actor, "name": "Public help", "key_prefix": "nyxid_ag",
        "key_hash": "00".repeat(32), "scopes": "read write", "is_active": true,
        "callback_url": format!("{}{callback}", server.uri()), "created_at": bson::DateTime::now(),
    }).await.unwrap();
    Mock::given(method("POST"))
        .and(path(callback))
        .respond_with(ResponseTemplate::new(202))
        .mount(server)
        .await;
    id
}

async fn wait_for_callbacks(state: &crate::AppState, bot: &ChannelBot, expected: u64) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let count = state
                .db
                .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
                .count_documents(doc! {"channel_bot_id": &bot.id, "callback_status": "delivered"})
                .await
                .unwrap();
            if count == expected {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("All agent callbacks must finish");
}

#[tokio::test]
async fn telegram_new_manager_public_channel_http_chat_reply_and_creation_end_to_end() {
    manager_channel_http_end_to_end(false).await;
}

#[tokio::test]
async fn telegram_new_manager_org_channel_http_chat_reply_and_creation_end_to_end() {
    manager_channel_http_end_to_end(true).await;
}

async fn manager_channel_http_end_to_end(org_owned: bool) {
    let trace = Arc::new(Mutex::new(TransactionTrace::default()));
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "telegram_manager_http",
        transaction_observer(trace.clone()),
    )
    .await;
    let (state, actor, server) = fixture_with_database(db).await;
    let target_org_id = if org_owned {
        use crate::models::org_membership::{MemberScopeSource, OrgRole};
        let org =
            crate::services::org_service::create_org_user(&state.db, "Manager channel", None, None)
                .await
                .unwrap();
        crate::services::org_service::create_membership(
            &state.db,
            &org.id,
            &actor,
            OrgRole::Admin,
            MemberScopeSource::Inherit,
            None,
        )
        .await
        .unwrap();
        Some(org.id)
    } else {
        None
    };
    let owner = target_org_id.as_deref().unwrap_or(&actor);
    super::manager_channel::bot_identity_api(&server, MANAGER).await;
    let (url, http_task) = start_http(&state, &server).await;
    let token = access_token(&state, &actor);
    let bots_url = format!("{url}/api/v1/channel-bots");
    let conversations_url = format!("{url}/api/v1/channel-conversations");
    let registration = json!({
        "platform": "telegram", "bot_token": MANAGER, "label": "Public manager",
        "target_org_id": target_org_id,
    });
    for (credential, expected) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some("nyxid_ag_denied"), StatusCode::FORBIDDEN),
    ] {
        let mut request = state.http_client.post(&bots_url).json(&registration);
        if let Some(credential) = credential {
            request = request.bearer_auth(credential);
        }
        response_json(request.send().await.unwrap(), expected).await;
    }
    assert!(server.received_requests().await.unwrap().is_empty());
    let created = response_json(
        state
            .http_client
            .post(&bots_url)
            .bearer_auth(&token)
            .json(&registration)
            .send()
            .await
            .unwrap(),
        StatusCode::CREATED,
    )
    .await;
    assert_eq!(created["credential_source"], "telegram_manager");
    assert_eq!(created["status"], "active");
    assert!(created.get("webhook_secret").is_none());
    let channel =
        crate::services::channel_bot_service::get_bot(&state.db, created["id"].as_str().unwrap())
            .await
            .unwrap();
    assert_eq!(channel.user_id, owner);
    assert!(channel.bot_token_encrypted.is_empty());
    assert!(channel.webhook_secret_hash.is_empty());
    let detail_url = format!("{bots_url}/{}", channel.id);
    let public_agent = add_agent(&state, owner, &server, "/public-agent").await;
    let exact_agent = add_agent(&state, owner, &server, "/exact-agent").await;
    let mut route_ids = Vec::new();
    for (chat, agent, default) in [("*", &public_agent, true), ("700", &exact_agent, false)] {
        let route = response_json(
            state
                .http_client
                .post(&conversations_url)
                .bearer_auth(&token)
                .json(&json!({
                    "channel_bot_id": channel.id, "agent_api_key_id": agent,
                    "platform_conversation_id": chat, "default_agent": default,
                    "target_org_id": target_org_id,
                }))
                .send()
                .await
                .unwrap(),
            StatusCode::CREATED,
        )
        .await;
        route_ids.push(route["id"].as_str().unwrap().to_owned());
    }
    Mock::given(method("POST"))
        .and(path(format!("/bot{MANAGER}/getWebhookInfo")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": {"url": ""}})),
        )
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    response_json(
        state
            .http_client
            .post(&bots_url)
            .bearer_auth(&token)
            .json(&registration)
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
    )
    .await;
    let bot_query: Vec<_> = target_org_id.iter().map(|org| ("org_id", org)).collect();
    let failed = response_json(
        state
            .http_client
            .get(&bots_url)
            .bearer_auth(&token)
            .query(&bot_query)
            .send()
            .await
            .unwrap(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(failed["total"], 1);
    assert_eq!(failed["bots"][0]["id"], channel.id);
    assert_eq!(failed["bots"][0]["status"], "failed");
    assert_eq!(failed["bots"][0]["webhook_registered"], false);

    let outsider = uuid::Uuid::new_v4().to_string();
    state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&outsider, crate::models::user::UserType::Person))
        .await
        .unwrap();
    let outsider_token = access_token(&state, &outsider);
    let denied = state
        .http_client
        .post(&bots_url)
        .bearer_auth(&outsider_token)
        .json(&registration)
        .send()
        .await
        .unwrap();
    assert!(denied.status().is_client_error());
    response_json(
        state
            .http_client
            .get(&detail_url)
            .bearer_auth(&outsider_token)
            .send()
            .await
            .unwrap(),
        StatusCode::NOT_FOUND,
    )
    .await;
    let mut retry = registration.clone();
    retry["label"] = json!("Do not replace saved label");
    let recovered = response_json(
        state
            .http_client
            .post(&bots_url)
            .bearer_auth(&token)
            .json(&retry)
            .send()
            .await
            .unwrap(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(recovered["id"], channel.id);
    assert_eq!(recovered["status"], "active");
    assert!(recovered.get("webhook_secret").is_none());
    let detail = response_json(
        state
            .http_client
            .get(&detail_url)
            .bearer_auth(&token)
            .send()
            .await
            .unwrap(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(detail["label"], "Public manager");
    assert_eq!(detail["status"], "active");
    assert_eq!(detail["webhook_registered"], true);
    assert_eq!(detail["conversations_count"], 2);
    assert_eq!(detail["user_id"], owner);
    let mut route_query = vec![("bot_id", channel.id.as_str())];
    if let Some(org) = target_org_id.as_deref() {
        route_query.push(("org_id", org));
    }
    let routes = response_json(
        state
            .http_client
            .get(&conversations_url)
            .bearer_auth(&token)
            .query(&route_query)
            .send()
            .await
            .unwrap(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(routes["total"], 2);
    for route in routes["conversations"].as_array().unwrap() {
        assert!(route_ids.contains(&route["id"].as_str().unwrap().to_owned()));
    }
    let inbound = format!("{url}/api/v1/webhooks/channel/telegram-new/manager");
    let reply_url = format!("{url}/api/v1/channel-relay/reply");
    let base = server.uri();
    let manager = service(&state, &base);
    let launch = response_json(
        state.http_client.post(format!("{bots_url}/telegram-new")).bearer_auth(&token)
            .json(&json!({"label": "Created while chatting", "auto_connect": true, "target_org_id": target_org_id}))
            .send().await.unwrap(), StatusCode::OK,
    ).await;
    let request_id = launch["request"]["id"].as_str().unwrap();
    let initial_request = manager.get(&actor, request_id).await.unwrap();
    let challenge = reqwest::Url::parse(launch["launch_url"].as_str().unwrap())
        .unwrap()
        .query_pairs()
        .find(|(name, _)| name == "start")
        .unwrap()
        .1
        .into_owned();
    let mut start = message(json!({"text": format!("/start {challenge}")}));
    start["update_id"] = json!(10);
    assert_eq!(
        state
            .http_client
            .post(&inbound)
            .headers(headers(&state).await)
            .json(&start)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );

    let pending = manager.get(&actor, request_id).await.unwrap();
    let pending_snapshot = bson::to_document(&pending).unwrap();
    let child_filter = doc! {"$or": [{"_id": request_id}, {"platform": {"$in": ["telegram", "telegram-new"]}, "platform_bot_id": "900"}]};
    let events = state
        .db
        .collection::<bson::Document>(crate::models::telegram_bot_request::MANAGED_BOTS);
    let event_filter = doc! {"manager_bot_id": pending.manager_bot_id, "telegram_bot_id": 900_i64};
    let mut management_event = None;

    for (index, chat, sender, kind) in [
        (11, 700, 700, "private"),
        (12, 701, 701, "private"),
        (13, -400, 702, "group"),
    ] {
        let update = json!({"update_id": index, "message": {
            "message_id": index, "from": {"id": sender, "is_bot": false}, "chat": {"id": chat, "type": kind},
            "date": Utc::now().timestamp(), "text": format!("Public help from {sender}"),
        }});
        assert_eq!(
            state
                .http_client
                .post(&inbound)
                .headers(headers(&state).await)
                .json(&update)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        if index == 12 {
            let management = json!({"update_id": 14, "managed_bot": {"user": {"id": 700, "is_bot": false}, "bot": bot()}});
            assert_eq!(
                state
                    .http_client
                    .post(&inbound)
                    .headers(headers(&state).await)
                    .json(&management)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::OK
            );
            management_event = events.find_one(event_filter.clone()).await.unwrap();
        }
    }
    // No additional awaited work between chat13 and creation15: keep the
    // original overlap with asynchronous ordinary-message admission.
    let mut expected_event = management_event.expect("management update14 must be recorded");
    let mut created = message(json!({"managed_bot_created": {"bot": bot()}}));
    created["update_id"] = json!(15);
    expected_event.insert("created_by", 700_i64);
    expected_event.insert("bot_username", bot()["username"].as_str().unwrap());
    expected_event.insert("observation_id", &pending.observation_id);
    expected_event.insert(
        "created_at",
        bson::DateTime::from_millis(created["message"]["date"].as_i64().unwrap() * 1_000),
    );
    // Reuse these exact bytes/date/update15 on redelivery and duplicate; never
    // replace the fixture, original request, creation proof or provider identity.
    let created_body = serde_json::to_vec(&created).unwrap();
    let mut definite_aborts = 0;
    // Ordinary chat admission writes the same owner user document as creation
    // authority fences. Either ownership type may definitely abort under overlap.
    for attempt in 0..3 {
        trace.lock().unwrap().outcomes.clear();
        let response = state
            .http_client
            .post(&inbound)
            .headers(headers(&state).await)
            .header("content-type", "application/json")
            .body(created_body.clone())
            .send()
            .await
            .unwrap();
        let status = response.status();
        let (outcomes, overflowed, pending_commands) = {
            let observations = trace.lock().unwrap();
            (
                observations.outcomes.clone(),
                observations.overflowed,
                observations.pending.len(),
            )
        };
        // Emit allowlisted operation AND commit evidence before any response
        // assertion/body decoding, including unexpected statuses and failures.
        eprintln!(
            "telegram_manager_creation_response attempt={attempt} org_owned={org_owned} update=15 status={status} overflowed={overflowed} pending={pending_commands} mongo={outcomes:?}"
        );
        assert!(!overflowed);
        assert_eq!(pending_commands, 0);
        assert!(!outcomes.iter().any(|outcome| outcome.unknown_commit));
        if status == StatusCode::OK {
            assert!(outcomes.iter().all(|outcome| outcome.succeeded));
            assert!(
                outcomes
                    .iter()
                    .any(|outcome| outcome.command == "commitTransaction")
            );
            break;
        }
        let envelope: Value = response.json().await.unwrap();
        assert_eq!(status, StatusCode::CONFLICT, "{envelope}");
        assert_eq!(envelope["error_code"], 1004, "{envelope}");
        assert_eq!(envelope["error"], "conflict", "{envelope}");
        assert_eq!(
            envelope["message"], "Conflict: Security state changed; restart the operation",
            "{envelope}"
        );
        let failures: Vec<_> = outcomes
            .iter()
            .filter(|outcome| !outcome.succeeded)
            .collect();
        assert_eq!(
            failures.len(),
            1,
            "conflict requires one correlated definite abort"
        );
        let failure = failures[0];
        assert_eq!(failure.update_id, 15);
        assert_eq!(failure.code, Some(112));
        assert!(failure.transient);
        assert!(failure.request_id > 0 && failure.transaction >= 0);
        assert!(
            outcomes
                .iter()
                .all(|outcome| outcome.session == failure.session
                    && outcome.transaction == failure.transaction)
        );
        assert!(
            match (failure.command, failure.collection, failure.statement) {
                ("update", "users", "actor_person_fence")
                | ("update", "platform_credentials", "telegram_creation_fence")
                | ("update", "telegram_bot_requests", "request_mutation") => true,
                ("update", "users", "owner_org_fence")
                | ("update", "org_memberships", "telegram_creation_fence") => org_owned,
                ("commitTransaction", "transaction", "commit") =>
                    outcomes.iter().any(|outcome| outcome.command == "update"
                        && outcome.statement == "request_mutation"
                        && outcome.succeeded),
                _ => false,
            },
            "unclassified creation conflict"
        );
        // Definite abort must leave the whole request (including revision,
        // context, expiry, actor/owner and start boundary) unchanged.
        let unchanged = manager.get(&actor, request_id).await.unwrap();
        assert!(
            bson::to_document(&unchanged).unwrap() == pending_snapshot,
            "aborted creation mutated the original request"
        );
        assert!(
            events
                .find_one(event_filter.clone())
                .await
                .unwrap()
                .unwrap()
                == expected_event,
            "creation provenance changed across abort/redelivery"
        );
        assert_eq!(
            events.count_documents(event_filter.clone()).await.unwrap(),
            1
        );
        assert_eq!(
            state
                .db
                .collection::<ChannelBot>(BOTS)
                .count_documents(child_filter.clone())
                .await
                .unwrap(),
            0
        );
        assert!(!server.received_requests().await.unwrap().iter().any(
            |request| request.url.path() == format!("/bot{MANAGER}/getManagedBotToken")
                || request.url.path() == format!("/bot{CHILD}/setWebhook")
        ));
        definite_aborts += 1;
        eprintln!(
            "telegram_manager_creation_definite_abort attempt={attempt} org_owned={org_owned} update=15 code=112 transient=true unknown_commit=false unchanged_request=true no_child_effect=true"
        );
        assert!(
            attempt < 2,
            "fresh creation did not settle after bounded redelivery"
        );
        // Preserve initial chat/creation overlap. Only after a proven abort,
        // await the already-issued callbacks' completion before redelivery.
        wait_for_callbacks(&state, &channel, 3).await;
    }

    let consumed = manager.get(&actor, request_id).await.unwrap();
    assert_eq!(consumed.status, Status::Ready);
    assert_eq!(consumed.telegram_bot_id, Some(900));
    let mut expected_request = pending_snapshot;
    expected_request.insert("status", "ready");
    expected_request.insert("telegram_bot_id", 900_i64);
    expected_request.insert("bot_username", bot()["username"].as_str().unwrap());
    expected_request.insert("manager_revision", 1_i64);
    expected_request.insert("revision", pending.revision + 1);
    assert!(
        bson::to_document(&consumed).unwrap() == expected_request,
        "creation must consume the same request exactly once"
    );
    assert!(
        events
            .find_one(event_filter.clone())
            .await
            .unwrap()
            .unwrap()
            == expected_event,
        "creation must preserve its original management provenance"
    );
    assert_eq!(
        events.count_documents(event_filter.clone()).await.unwrap(),
        1
    );
    assert_eq!(
        state
            .http_client
            .post(&inbound)
            .headers(headers(&state).await)
            .header("content-type", "application/json")
            .body(created_body)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK,
    );
    assert!(
        bson::to_document(&manager.get(&actor, request_id).await.unwrap()).unwrap()
            == expected_request,
        "duplicate creation changed the consumed request"
    );
    assert!(
        events
            .find_one(event_filter.clone())
            .await
            .unwrap()
            .unwrap()
            == expected_event,
        "duplicate creation changed immutable provenance"
    );
    assert_eq!(events.count_documents(event_filter).await.unwrap(), 1);
    provider_connection(&server, request_id, 200).await;
    manager.complete_pending_creations().await.unwrap();
    manager.complete_pending_creations().await.unwrap();
    let child = response_json(
        state
            .http_client
            .get(format!("{bots_url}/telegram-new/requests/{request_id}"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(child["status"], "connected");
    assert_eq!(child["owner_user_id"], owner);
    let saved = manager.get(&actor, request_id).await.unwrap();
    assert_eq!(saved.start_update_id, Some(10));
    assert_eq!(saved.telegram_user_id, Some(700));
    assert_eq!(saved.telegram_bot_id, Some(900));
    assert_eq!(saved.id, initial_request.id);
    assert_eq!(saved.actor_user_id, initial_request.actor_user_id);
    assert_eq!(saved.owner_user_id, initial_request.owner_user_id);
    assert_eq!(saved.created_at, initial_request.created_at);
    assert_eq!(
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .count_documents(child_filter.clone())
            .await
            .unwrap(),
        1
    );
    assert!(!trace.lock().unwrap().overflowed);
    wait_for_callbacks(&state, &channel, 3).await;

    let callbacks: Vec<_> = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| matches!(request.url.path(), "/public-agent" | "/exact-agent"))
        .collect();
    assert_eq!(callbacks.len(), 3, "Setup updates must not reach agents");
    let mut payloads = Vec::new();
    for callback in &callbacks {
        assert!(!callback.headers.contains_key("x-nyxid-user-token"));
        assert!(callback.headers.contains_key("x-nyxid-callback-token"));
        let payload = callback.body_json::<Value>().unwrap();
        let claims = crate::crypto::jwt::validate_relay_reply_token(
            &state.jwt_keys,
            &state.config,
            payload["reply_token"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(claims.inbound_message_id, payload["message_id"]);
        assert_eq!(
            payload["agent"]["api_key_id"].as_str().unwrap(),
            if callback.url.path() == "/exact-agent" {
                exact_agent.as_str()
            } else {
                public_agent.as_str()
            }
        );
        payloads.push(payload);
    }
    assert_eq!(
        callbacks
            .iter()
            .filter(|r| r.url.path() == "/exact-agent")
            .count(),
        1
    );

    // A reply token cannot target another sender's message, and a rejected
    // attempt must not consume the token for its legitimate message.
    let wrong = state
        .http_client
        .post(&reply_url)
        .bearer_auth(payloads[0]["reply_token"].as_str().unwrap())
        .json(
            &json!({"message_id": payloads[1]["message_id"], "reply": {"text": "wrong recipient"}}),
        )
        .send()
        .await
        .unwrap();
    assert!(wrong.status().is_client_error());
    for payload in payloads.iter().take(2) {
        let response = state.http_client.post(&reply_url).bearer_auth(payload["reply_token"].as_str().unwrap())
            .json(&json!({"message_id": payload["message_id"], "reply": {"text": "Public assistant response"}})).send().await.unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let sent = server.received_requests().await.unwrap();
    assert_eq!(
        sent.iter()
            .filter(|r| r.url.path() == format!("/bot{MANAGER}/getManagedBotToken"))
            .count(),
        1
    );
    assert_eq!(
        sent.iter()
            .filter(|r| r.url.path() == format!("/bot{CHILD}/setWebhook"))
            .count(),
        1
    );
    eprintln!(
        "telegram_manager_creation_complete org_owned={org_owned} update=15 definite_aborts={definite_aborts} same_event_ready=true duplicate_stable=true child_count=1 token_calls=1 webhook_calls=1"
    );
    let replies: Vec<_> = sent
        .iter()
        .filter(|r| r.url.path() == format!("/bot{MANAGER}/sendMessage"))
        .filter_map(|r| r.body_json::<Value>().ok())
        .filter(|v| v["text"] == "Public assistant response")
        .collect();
    assert_eq!(replies.len(), 2);
    for payload in payloads.iter().take(2) {
        assert!(
            replies
                .iter()
                .any(|r| r["chat_id"].to_string().trim_matches('"')
                    == payload["conversation"]["platform_id"].as_str().unwrap())
        );
    }
    assert!(
        !sent
            .iter()
            .any(|r| r.url.path() == format!("/bot{MANAGER}/setWebhook"))
    );

    // The unchanged normal Telegram path still supplies scoped owner authority.
    let mut ordinary = channel.clone();
    ordinary.credential_source = "user".into();
    let adapter = crate::services::channel_adapters::telegram::TelegramAdapter::new();
    let messages = adapter
        .parse_inbound(&serde_json::to_vec(&message(json!({"text": "Ordinary bot"}))).unwrap())
        .await
        .unwrap();
    crate::services::channel_inbound_service::process_inbound_messages(
        (&state).into(),
        &ordinary,
        &adapter,
        &messages,
    )
    .await
    .unwrap();
    assert!(server.received_requests().await.unwrap().iter().any(|r| r.url.path() == "/exact-agent" && r.headers.contains_key("x-nyxid-user-token")));

    assert_eq!(
        state
            .http_client
            .delete(&detail_url)
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    let routes = response_json(
        state
            .http_client
            .get(&conversations_url)
            .bearer_auth(&token)
            .query(&route_query)
            .send()
            .await
            .unwrap(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(routes["total"], 0);
    let rejected = state
        .http_client
        .post(&reply_url)
        .bearer_auth(payloads[2]["reply_token"].as_str().unwrap())
        .json(
            &json!({"message_id": payloads[2]["message_id"], "reply": {"text": "after deletion"}}),
        )
        .send()
        .await
        .unwrap();
    assert!(rejected.status().is_client_error());
    assert!(manager.manager().await.is_ok());
    let mut recover = message(json!({"text": "/recover @CustomerBot"}));
    recover["update_id"] = json!(16);
    assert_eq!(
        state
            .http_client
            .post(&inbound)
            .headers(headers(&state).await)
            .json(&recover)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert!(
        !server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path() == format!("/bot{MANAGER}/deleteWebhook"))
    );
    http_task.abort();
}

#[tokio::test]
async fn telegram_new_manager_ordinary_lookup_failure_isolated_from_control_failure() {
    let (state, actor, server) = fixture().await;
    let channel = register_manager_channel(&state, &actor, &server).await;
    let (url, http_task) = start_http(&state, &server).await;
    let inbound = format!("{url}/api/v1/webhooks/channel/telegram-new/manager");
    state
        .db
        .collection::<bson::Document>(BOTS)
        .update_one(
            doc! {"_id": &channel.id},
            doc! {"$set": {"label": ["invalid model field"]}},
        )
        .await
        .unwrap();
    let response = state
        .http_client
        .post(&inbound)
        .headers(headers(&state).await)
        .json(&message(json!({"text": "hello"})))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let invalid_auth = state
        .http_client
        .post(&inbound)
        .json(&message(json!({"text": "hello"})))
        .send()
        .await
        .unwrap();
    assert!(invalid_auth.status().is_client_error());
    state.db.collection::<bson::Document>(crate::models::telegram_bot_request::MANAGED_BOTS).insert_one(doc! {
        "_id": uuid::Uuid::new_v4().to_string(), "manager_bot_id": 100_i64, "telegram_bot_id": 900_i64, "revision": "invalid", "update_ids": [],
    }).await.unwrap();
    let control = state
        .http_client
        .post(&inbound)
        .headers(headers(&state).await)
        .json(&json!({"update_id": 99, "managed_bot": {"user": {"id": 700}, "bot": bot()}}))
        .send()
        .await
        .unwrap();
    assert!(control.status().is_server_error());
    http_task.abort();
}

#[tokio::test]
async fn telegram_new_creation_recovery_http_authentication_and_middleware_boundary() {
    let (state, actor, server) = fixture().await;
    let base = server.uri();
    let service = service(&state, &base);
    let (request, launch) = service
        .begin(&actor, &actor, "Scoped recovery", true)
        .await
        .unwrap();
    let challenge = reqwest::Url::parse(&launch)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "start")
        .unwrap()
        .1
        .to_string();
    webhook(
        &service,
        &headers(&state).await,
        message(json!({"text":format!("/start {challenge}")})),
    )
    .await;
    let (url, task) = start_http(&state, &server).await;
    let endpoint = format!(
        "{url}/api/v1/channel-bots/telegram-new/requests/{}/creation-link",
        request.id
    );
    let client = reqwest::Client::new();
    assert_eq!(
        client.post(&endpoint).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let denied = super::super::key_service::create_api_key(
        &state.db,
        &actor,
        "recovery denied",
        "read write",
        None,
        None,
        Some(&[]),
        Some(&[]),
        Some(false),
        Some(false),
        Some(false),
        None,
        None,
        Some("generic"),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        client
            .post(&endpoint)
            .header("X-API-Key", denied.full_key.as_str())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let response = client
        .post(&endpoint)
        .bearer_auth(access_token(&state, &actor))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["request"]["id"], request.id);
    assert!(
        body["creation_url"]
            .as_str()
            .unwrap()
            .starts_with("https://t.me/newbot/NyxSetupBot/")
    );
    task.abort();
}

mod production_router_recovery {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
        response::Response,
    };
    use tower::ServiceExt;

    fn production_router(state: crate::AppState) -> Router {
        crate::routes::build_router_with_state(state.clone())
            .1
            .with_state(state)
    }

    async fn post_recovery(
        app: &Router,
        endpoint: &str,
        credential: Option<(&str, &str)>,
        body: Body,
    ) -> Response {
        let mut request = Request::builder().method("POST").uri(endpoint);
        if let Some((header, value)) = credential {
            request = request.header(header, value);
        }
        app.clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap()
    }

    async fn assert_denied(
        response: Response,
        status: StatusCode,
        error: &str,
        code: u32,
        credential_type: Option<&str>,
        protected: &[&str],
    ) {
        assert_eq!(response.status(), status);
        let bytes = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        // Boolean-only diagnostics: never print a response or credential if a
        // disclosure regression puts private material in an unexpected field.
        assert!(body["error"] == error, "unexpected refusal category");
        assert!(body["error_code"] == code, "unexpected refusal code");
        assert!(body.get("creation_url").is_none());
        assert!(body.get("request").is_none());
        if let Some(credential_type) = credential_type {
            assert!(body["details"]["reason"] == "credential_type_unsupported");
            assert!(body["details"]["credential_type"] == credential_type);
            assert!(body["details"]["accepted"] == json!(["user_session"]));
        }
        let text = std::str::from_utf8(&bytes).unwrap();
        for value in protected {
            assert!(
                !text.contains(value),
                "refusal disclosed protected fixture material"
            );
        }
    }

    async fn saved_request(state: &crate::AppState, id: &str) -> bson::Document {
        state
            .db
            .collection::<bson::Document>(REQUESTS)
            .find_one(doc! {"_id": id})
            .await
            .unwrap()
            .expect("the seeded request must remain present")
    }

    async fn assert_effects(
        state: &crate::AppState,
        server: &MockServer,
        id: &str,
        expected: &bson::Document,
        provider_calls: usize,
    ) {
        assert!(
            saved_request(state, id).await == *expected,
            "unexpected request or recovery issuance mutation"
        );
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(REQUESTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(BOTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            provider_calls
        );
    }

    #[tokio::test]
    async fn telegram_new_creation_recovery_production_router_enforces_human_owner_boundary() {
        // This fixture requires a real writable transaction-capable database;
        // only the initial private Telegram handoff uses the local provider.
        let (state, actor, server) = fixture().await;
        let base = server.uri();
        let service = service(&state, &base);
        let (request, launch) = service
            .begin(&actor, &actor, "Scoped recovery", true)
            .await
            .unwrap();
        let challenge = reqwest::Url::parse(&launch)
            .unwrap()
            .query_pairs()
            .find(|(key, _)| key == "start")
            .unwrap()
            .1
            .into_owned();
        webhook(
            &service,
            &headers(&state).await,
            message(json!({"text": format!("/start {challenge}")})),
        )
        .await;
        let bound = service.get(&actor, &request.id).await.unwrap();
        assert_eq!(bound.status, Status::WaitingBot);
        assert_eq!(bound.telegram_user_id, Some(700));
        assert_eq!(bound.start_update_id, Some(1));
        let before = saved_request(&state, &request.id).await;
        let provider_calls = server.received_requests().await.unwrap().len();
        assert_eq!(provider_calls, 2);

        let foreign = uuid::Uuid::new_v4().to_string();
        state
            .db
            .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(test_user(&foreign, crate::models::user::UserType::Person))
            .await
            .unwrap();
        let owner_token = access_token(&state, &actor);
        let foreign_token = access_token(&state, &foreign);
        let expired_token = crate::crypto::jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &uuid::Uuid::parse_str(&actor).unwrap(),
            "read write",
            None,
            Some(-3600),
            None,
            None,
            None,
        )
        .unwrap();
        let delegated_token = crate::crypto::jwt::generate_delegated_access_token(
            &state.jwt_keys,
            &state.config,
            &uuid::Uuid::parse_str(&actor).unwrap(),
            "account:read",
            "telegram-router-recovery",
            600,
            None,
        )
        .unwrap();
        let key = crate::services::key_service::create_api_key(
            &state.db,
            &actor,
            "recovery route credential",
            "proxy",
            None,
            None,
            Some(&[]),
            Some(&[]),
            Some(false),
            Some(false),
            Some(false),
            None,
            None,
            Some("generic"),
            None,
        )
        .await
        .unwrap();
        let (account, service_secret) =
            crate::services::service_account_service::create_service_account(
                &state.db,
                "telegram-router-recovery",
                None,
                "proxy",
                &[],
                None,
                &actor,
            )
            .await
            .unwrap();
        let service_token =
            crate::services::service_account_service::authenticate_client_credentials(
                &state.db,
                &state.config,
                &state.jwt_keys,
                &account.client_id,
                &service_secret,
                Some("proxy"),
            )
            .await
            .unwrap()
            .access_token;
        let relay_token = crate::crypto::jwt::generate_relay_access_token(
            &state.jwt_keys,
            &state.config,
            &uuid::Uuid::parse_str(&actor).unwrap(),
            "proxy",
            None,
            &crate::crypto::jwt::RelayAgentScope {
                api_key_id: key.id.clone(),
                api_key_name: key.name.clone(),
                allowed_service_ids: vec![],
                allowed_node_ids: vec![],
                allow_all_services: false,
                allow_all_nodes: false,
            },
        )
        .unwrap();
        let owner = format!("Bearer {owner_token}");
        let foreign = format!("Bearer {foreign_token}");
        let expired = format!("Bearer {expired_token}");
        let delegated = format!("Bearer {delegated_token}");
        let api_key = format!("Bearer {}", key.full_key);
        let service_account = format!("Bearer {service_token}");
        let relay = format!("Bearer {relay_token}");
        let protected = [
            "t.me/",
            "creation_url",
            request.id.as_str(),
            actor.as_str(),
            "Scoped recovery",
            challenge.as_str(),
            MANAGER,
            CHILD,
            key.full_key.as_str(),
            service_secret.as_str(),
            owner_token.as_str(),
            foreign_token.as_str(),
            expired_token.as_str(),
            delegated_token.as_str(),
            service_token.as_str(),
            relay_token.as_str(),
            before.get_str("challenge_hash").unwrap(),
        ];

        // No test route, AuthUser extension, or credential rejection layer is
        // substituted: these requests enter the actual production router.
        let app = production_router(state.clone());
        let endpoint = format!(
            "/api/v1/channel-bots/telegram-new/requests/{}/creation-link",
            request.id
        );
        for (credential, status, error, code, credential_type) in [
            (None, StatusCode::UNAUTHORIZED, "unauthorized", 1001, None),
            (
                Some(("authorization", foreign.as_str())),
                StatusCode::NOT_FOUND,
                "not_found",
                1003,
                None,
            ),
            (
                Some(("authorization", expired.as_str())),
                StatusCode::UNAUTHORIZED,
                "token_expired",
                2001,
                None,
            ),
            (
                Some(("authorization", delegated.as_str())),
                StatusCode::FORBIDDEN,
                "forbidden",
                1002,
                Some("delegated"),
            ),
            (
                Some(("x-api-key", key.full_key.as_str())),
                StatusCode::FORBIDDEN,
                "forbidden",
                1002,
                Some("api_key"),
            ),
            (
                Some(("authorization", api_key.as_str())),
                StatusCode::FORBIDDEN,
                "forbidden",
                1002,
                Some("api_key"),
            ),
            (
                Some(("authorization", service_account.as_str())),
                StatusCode::FORBIDDEN,
                "forbidden",
                1002,
                Some("service_account"),
            ),
            (
                Some(("authorization", relay.as_str())),
                StatusCode::FORBIDDEN,
                "forbidden",
                1002,
                Some("relay"),
            ),
        ] {
            assert_denied(
                post_recovery(&app, &endpoint, credential, Body::empty()).await,
                status,
                error,
                code,
                credential_type,
                &protected,
            )
            .await;
            assert_effects(&state, &server, &request.id, &before, provider_calls).await;
        }
        assert_denied(
            post_recovery(
                &app,
                &endpoint,
                Some(("authorization", &owner)),
                Body::from("{}"),
            )
            .await,
            StatusCode::BAD_REQUEST,
            "bad_request",
            1000,
            None,
            &protected,
        )
        .await;
        assert_effects(&state, &server, &request.id, &before, provider_calls).await;

        let response = post_recovery(
            &app,
            &endpoint,
            Some(("authorization", &owner)),
            Body::empty(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["referrer-policy"], "no-referrer");
        let bytes = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(body["request"]["id"] == request.id);
        assert!(body["request"]["owner_user_id"] == actor);
        assert!(body["request"]["label"] == "Scoped recovery");
        assert!(body["request"]["status"] == "waiting_bot");
        assert!(body["request"]["revision"] == bound.revision);
        assert!(body["request"]["expires_at"] == bound.expires_at.to_rfc3339());
        assert!(body["request"]["can_open_creation_form"] == true);
        assert!(
            body["creation_url"]
                == "https://t.me/newbot/NyxSetupBot/scoped_recovery_bot?name=Scoped+recovery"
        );
        for field in [
            "challenge_hash",
            "consent_hash",
            "creation_context",
            "telegram_user_id",
        ] {
            assert!(body["request"].get(field).is_none());
        }
        let mut issued = before.clone();
        issued.insert(
            "creation_link_generation",
            before.get_i64("creation_link_generation").unwrap_or(0) + 1,
        );
        assert_effects(&state, &server, &request.id, &issued, provider_calls).await;

        // Expiring a request legitimately closes it in get()/expire(). Refuse
        // issuance without mistaking that existing lifecycle write for a leak.
        state.db.collection::<bson::Document>(REQUESTS).update_one(
            doc! {"_id": &request.id},
            doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}},
        ).await.unwrap();
        let mut expected_expired = saved_request(&state, &request.id).await;
        assert_denied(
            post_recovery(
                &app,
                &endpoint,
                Some(("authorization", &owner)),
                Body::empty(),
            )
            .await,
            StatusCode::CONFLICT,
            "conflict",
            1004,
            None,
            &protected,
        )
        .await;
        let expired = saved_request(&state, &request.id).await;
        let purge_after = *expired.get_datetime("purge_after").unwrap();
        assert!(purge_after.to_chrono() > Utc::now() + Duration::hours(23));
        assert!(purge_after.to_chrono() < Utc::now() + Duration::hours(25));
        expected_expired.insert("active", false);
        expected_expired.insert("status", "expired");
        expected_expired.insert("revision", issued.get_i64("revision").unwrap() + 1);
        expected_expired.insert("purge_after", purge_after);
        expected_expired.remove("consent_hash");
        assert_effects(
            &state,
            &server,
            &request.id,
            &expected_expired,
            provider_calls,
        )
        .await;
        state.db.drop().await.unwrap();
    }
}
