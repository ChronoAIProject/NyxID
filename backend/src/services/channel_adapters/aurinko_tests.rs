use super::*;
use crate::models::{
    api_key::ApiKey,
    channel_email::{EmailSend, EmailSubscription, SENDS, SUBSCRIPTIONS},
};
use crate::services::{channel_bot_service as bots, channel_routing_service as routes};
use axum::{
    Json, Router, body::Bytes, extract::State, http::Uri, response::IntoResponse, routing::any,
};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::Mutex;

const TOKEN: &str = "test-account-token";
const SECRET: &str = "test-signing-secret";

fn signed(body: &[u8], timestamp: i64) -> HeaderMap {
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(format!("v0:{timestamp}:").as_bytes());
    mac.update(body);
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-aurinko-request-timestamp",
        timestamp.to_string().parse().unwrap(),
    );
    headers.insert(
        "x-aurinko-signature",
        hex::encode(mac.finalize().into_bytes()).parse().unwrap(),
    );
    headers
}
fn mail(id: &str) -> Value {
    json!({"id": id, "threadId": "thread-1", "receivedAt": (Utc::now() + chrono::Duration::minutes(1)).to_rfc3339(),
        "from": {"address": "sender@example.com", "name": "Sender"}, "to": [{"address": "mailbox@example.com"}],
        "replyTo": [{"address": "reply@example.com"}], "sysLabels": ["inbox"], "subject": "Private subject",
        "body": "Private message body", "internetHeaders": [], "omitted": []})
}
fn account(id: u64) -> Value {
    json!({"id": id, "email": "mailbox@example.com", "tokenStatus": "active", "authScopes": ["Mail.Read", "Mail.Send"]})
}

struct Mock {
    messages: BTreeMap<String, Value>,
    statuses: BTreeMap<String, u16>,
    delays: BTreeMap<String, u64>,
    subscriptions: Vec<Value>,
    callbacks: Vec<Value>,
    replies: Vec<Value>,
    requests: Vec<(String, String)>,
    callback_status: u16,
    send_status: u16,
    send_body: Value,
    handshake: Option<(crate::AppState, String)>,
}
impl Default for Mock {
    fn default() -> Self {
        Self {
            messages: BTreeMap::new(),
            statuses: BTreeMap::new(),
            delays: BTreeMap::new(),
            subscriptions: vec![],
            callbacks: vec![],
            replies: vec![],
            requests: vec![],
            callback_status: 202,
            send_status: 200,
            send_body: json!({"status":"Ok", "id":"sent-1"}),
            handshake: None,
        }
    }
}
async fn mock_request(
    State(shared): State<Arc<Mutex<Mock>>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let path = uri.path();
    let mut mock = shared.lock().await;
    mock.requests.push((method.to_string(), uri.to_string()));
    if path == "/callback" {
        mock.callbacks.push(serde_json::from_slice(&body).unwrap());
        return StatusCode::from_u16(mock.callback_status)
            .unwrap()
            .into_response();
    }
    if headers.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer revoked") {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if path == "/v1/account" {
        let id = if headers["authorization"] == "Bearer other-account" {
            99
        } else {
            42
        };
        return Json(account(id)).into_response();
    }
    if path == "/v1/subscriptions" {
        if method == Method::GET {
            return Json(json!({"records": mock.subscriptions, "done": true})).into_response();
        }
        let input: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(input["detailLevel"], "status");
        assert_eq!(input["filters"], json!(["withoutDrafts"]));
        let handshake = mock.handshake.clone();
        drop(mock);
        if let Some((state, bot_id)) = handshake {
            let challenge = crate::handlers::channel_webhooks::channel_webhook(
                State(state),
                axum::extract::Path(("aurinko".into(), bot_id)),
                "/?validationToken=challenge%20text%20%2B%20%2F"
                    .parse()
                    .unwrap(),
                signed(b"", Utc::now().timestamp()),
                Bytes::new(),
            )
            .await;
            if challenge.status() != StatusCode::OK {
                return StatusCode::BAD_REQUEST.into_response();
            }
            assert_eq!(challenge.headers()["content-type"], "text/plain");
            assert_eq!(
                axum::body::to_bytes(challenge.into_body(), 4096)
                    .await
                    .unwrap(),
                "challenge text + /"
            );
        }
        let mut mock = shared.lock().await;
        let id = 100 + mock.subscriptions.len();
        let value = json!({"id": id, "active":true, "resource":"/email/messages", "notificationUrl": input["notificationUrl"], "detailLevel":"status", "filters":["withoutDrafts"]});
        mock.subscriptions.push(value.clone());
        return Json(value).into_response();
    }
    if let Some(id) = path.strip_prefix("/v1/subscriptions/") {
        let id: u64 = id.parse().unwrap();
        if method == Method::DELETE {
            mock.subscriptions.retain(|row| row["id"] != id);
            return StatusCode::OK.into_response();
        }
    }
    if let Some(id) = path.strip_prefix("/v1/email/messages/") {
        if id.ends_with("/reply") {
            mock.replies.push(serde_json::from_slice(&body).unwrap());
            return (
                StatusCode::from_u16(mock.send_status).unwrap(),
                Json(mock.send_body.clone()),
            )
                .into_response();
        }
        let delay = mock.delays.get(id).copied().unwrap_or(0);
        let status = *mock.statuses.get(id).unwrap_or(&200);
        let value = mock.messages.get(id).cloned().unwrap_or_else(|| mail(id));
        assert!(
            uri.query()
                .unwrap_or_default()
                .contains("requireThreadId=true")
        );
        drop(mock);
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        return (StatusCode::from_u16(status).unwrap(), Json(value)).into_response();
    }
    StatusCode::NOT_FOUND.into_response()
}
struct Fixture {
    state: crate::AppState,
    adapter: AurinkoAdapter,
    bot: ChannelBot,
    mock: Arc<Mutex<Mock>>,
    server: tokio::task::JoinHandle<()>,
    key: ApiKey,
    route: crate::models::channel_conversation::ChannelConversation,
    url: String,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        let db = crate::test_utils::connect_test_database("aurinko")
            .await
            .expect("Aurinko tests require NYXID_TEST_DATABASE_URL and a real MongoDB");
        ensure_indexes(&db).await.unwrap();
        crate::services::coordination_service::ensure_indexes(&db)
            .await
            .unwrap();
        let state = crate::test_utils::test_app_state(db.clone());
        let mock = Arc::new(Mutex::new(Mock::default()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .fallback(any(mock_request))
            .with_state(mock.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let adapter = AurinkoAdapter {
            origin: Some(origin.clone()),
        };
        let owner = uuid::Uuid::new_v4().to_string();
        let fields = RegistrationValues(BTreeMap::from([
            ("bot_token", TOKEN),
            ("app_secret", SECRET),
        ]));
        let created = bots::create_bot(
            &db,
            &state.config,
            &state.encryption_keys,
            &state.http_client,
            &adapter,
            &owner,
            "Mailbox",
            &fields,
        )
        .await
        .unwrap();
        mock.lock().await.handshake = Some((state.clone(), created.bot.id.clone()));
        let url = format!(
            "https://nyxid.test/api/v1/webhooks/channel/aurinko/{}",
            created.bot.id
        );
        bots::register_webhook(
            &db,
            &state.http_client,
            &adapter,
            &created.bot.id,
            TOKEN,
            &url,
            "unused",
        )
        .await
        .unwrap();
        let bot = bots::get_bot(&db, &created.bot.id).await.unwrap();
        let key: ApiKey = bson::from_document(doc! { "_id": uuid::Uuid::new_v4().to_string(), "user_id": &owner,
            "name":"agent", "key_prefix":"nyxid_ag", "key_hash":"ab".repeat(32), "scopes":"read write",
            "is_active":true, "created_at":bson::DateTime::now(), "callback_url": format!("{origin}/callback"),
            "allow_all_services":false, "allowed_service_ids":["only-allowed-service"], "allow_all_nodes":false }).unwrap();
        db.collection::<ApiKey>(crate::models::api_key::COLLECTION_NAME)
            .insert_one(&key)
            .await
            .unwrap();
        let route = routes::create_conversation(
            &db,
            &owner,
            Some(&bot.id),
            "aurinko",
            "*",
            "private",
            None,
            &key.id,
            true,
            false,
        )
        .await
        .unwrap();
        Self {
            state,
            adapter,
            bot,
            mock,
            server,
            key,
            route,
            url,
        }
    }
    fn context(&self) -> IngressContext<'_> {
        IngressContext {
            db: &self.state.db,
            config: &self.state.config,
            jwt_keys: &self.state.jwt_keys,
            encryption_keys: &self.state.encryption_keys,
            http: &self.state.http_client,
            rate_limiter: &self.state.per_channel_event_limiter,
        }
    }
    async fn event(&self, ids: &[&str]) -> AppResult<Option<String>> {
        let body = serde_json::to_vec(&json!({"accountId":42, "subscription":100, "resource":"/email/messages",
            "payloads": ids.iter().map(|id| json!({"id": id, "changeType":"created"})).collect::<Vec<_>>()})).unwrap();
        self.adapter
            .receive(
                &self.context(),
                &self.bot.id,
                &signed(&body, Utc::now().timestamp()),
                &HashMap::new(),
                &body,
            )
            .await
    }
    async fn original(&self) -> ChannelMessage {
        self.state
            .db
            .collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME)
            .find_one(doc! { "channel_bot_id": &self.bot.id })
            .await
            .unwrap()
            .unwrap()
    }
    async fn reply(
        &self,
        original: &ChannelMessage,
        metadata: Option<Value>,
    ) -> AppResult<Option<String>> {
        self.adapter
            .send_bound_reply_outcome(
                &self.state.db,
                &self.state.http_client,
                &self.bot,
                original,
                &BotCredentials {
                    billing: None,
                    token: TOKEN,
                    platform_bot_id: Some("42"),
                    platform_secrets: None,
                },
                original.platform_conversation_id.as_deref().unwrap(),
                &OutboundReply {
                    attachments: vec![],
                    text: Some("Reply body".into()),
                    reply_to_platform_message_id: original.platform_message_id.clone(),
                    metadata,
                },
            )
            .await?
            .into_result()
    }
}

#[tokio::test]
async fn aurinko_signed_pending_handshake_tamper_account_and_subscription_binding() {
    let f = Fixture::new().await; // subscribe invokes the real signed pending HTTP handler
    let body =
        br#"{ "accountId":42, "subscription":100, "resource":"/email/messages", "payloads":[] }"#;
    let headers = signed(body, Utc::now().timestamp());
    assert!(
        f.adapter
            .receive(&f.context(), &f.bot.id, &headers, &HashMap::new(), body)
            .await
            .is_ok()
    );
    assert!(
        f.adapter
            .receive(&f.context(), &f.bot.id, &headers, &HashMap::new(), b"{}")
            .await
            .is_err()
    );
    for (account_id, subscription) in [(99, 100), (42, 999)] {
        let bad = serde_json::to_vec(&json!({"accountId":account_id,"subscription":subscription,"resource":"/email/messages","payloads":[]})).unwrap();
        assert!(matches!(
            f.adapter
                .receive(
                    &f.context(),
                    &f.bot.id,
                    &signed(&bad, Utc::now().timestamp()),
                    &HashMap::new(),
                    &bad
                )
                .await,
            Err(AppError::ChannelWebhookVerificationFailed(_))
        ));
    }
    assert!(
        verify_signature(
            &f.bot,
            SECRET,
            &signed(body, Utc::now().timestamp() + 301),
            body,
            Utc::now().timestamp()
        )
        .is_err()
    );
    let mut old_bot = f.bot.clone();
    old_bot.created_at = Utc::now() - chrono::Duration::days(2);
    assert!(
        verify_signature(
            &old_bot,
            SECRET,
            &signed(body, Utc::now().timestamp() - 3600),
            body,
            Utc::now().timestamp()
        )
        .is_ok()
    );
}

#[tokio::test]
async fn aurinko_filters_threads_and_automation_without_retaining_body() {
    let f = Fixture::new().await;
    let original = mail("one");
    let normalized = normalize(&f.bot, &account(42), &original).unwrap().unwrap();
    assert_eq!(
        normalized.text.as_deref(),
        Some("Subject: Private subject\n\nPrivate message body")
    );
    for label in ["sent", "draft", "junk", "trash"] {
        let mut value = original.clone();
        value["sysLabels"] = json!([label]);
        assert!(normalize(&f.bot, &account(42), &value).unwrap().is_none());
    }
    for (name, value) in [
        ("Auto-Submitted", "auto-replied"),
        ("List-Id", "list"),
        ("Return-Path", "<>"),
        ("X-NyxID-Auto-Reply", "true"),
        (
            "Content-Type",
            "multipart/report; report-type=delivery-status",
        ),
    ] {
        let mut msg = original.clone();
        msg["internetHeaders"] = json!([{"name":name,"value":value}]);
        assert!(normalize(&f.bot, &account(42), &msg).unwrap().is_none());
    }
    let mut msg = original.clone();
    msg["from"]["address"] = json!("mailbox@example.com");
    assert!(normalize(&f.bot, &account(42), &msg).unwrap().is_none());
    msg = original.clone();
    msg["receivedAt"] = json!((f.bot.created_at - chrono::Duration::days(1)).to_rfc3339());
    assert!(normalize(&f.bot, &account(42), &msg).unwrap().is_none());
    msg = original.clone();
    msg["threadId"] = Value::Null;
    assert!(normalize(&f.bot, &account(42), &msg).is_err());
    msg = original.clone();
    msg["threadId"] = json!("long".repeat(500));
    let long = normalize(&f.bot, &account(42), &msg).unwrap().unwrap();
    assert!(long.conversation_id.len() < 256);
    assert_ne!(long.conversation_id, normalized.conversation_id);
    f.event(&["one"]).await.unwrap();
    let stored = f
        .state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .find_one(doc! {"channel_bot_id":&f.bot.id})
        .await
        .unwrap()
        .unwrap();
    let wire = format!("{stored:?}");
    assert!(!wire.contains("Private"));
    for key in ["text", "body", "subject", "raw_data", "raw_platform_data"] {
        assert!(!stored.contains_key(key));
    }
    assert!(stored.get_array("attachments").unwrap().is_empty());
    let callback = f.mock.lock().await.callbacks[0].clone();
    assert_eq!(
        callback["conversation"]["platform_id"],
        normalized.conversation_id
    );
    assert!(!callback.to_string().contains(TOKEN));
    assert!(!callback.to_string().contains(SECRET));
    let route = f
        .state
        .db
        .collection::<crate::models::channel_conversation::ChannelConversation>(
            crate::models::channel_conversation::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&f.route.id})
        .await
        .unwrap()
        .unwrap();
    assert!(route.last_message_at.is_some());
}

#[tokio::test]
async fn aurinko_partial_batch_retries_reuse_uuid_and_committed_messages_dedup() {
    let f = Fixture::new().await;
    f.mock.lock().await.statuses.insert("retry".into(), 429);
    assert!(f.event(&["good", "retry"]).await.is_err());
    assert_eq!(f.mock.lock().await.callbacks.len(), 1);
    f.mock.lock().await.statuses.remove("retry");
    f.event(&["good", "retry"]).await.unwrap();
    assert_eq!(f.mock.lock().await.callbacks.len(), 2);
    f.event(&["good", "retry"]).await.unwrap();
    assert_eq!(f.mock.lock().await.callbacks.len(), 2);
    f.mock.lock().await.callback_status = 500;
    assert!(f.event(&["lost-ack"]).await.is_err());
    let before = f.mock.lock().await.callbacks.last().unwrap()["message_id"].clone();
    // Replace the default route and retry the same provider message.
    routes::create_conversation(
        &f.state.db,
        &f.bot.user_id,
        Some(&f.bot.id),
        "aurinko",
        "*",
        "private",
        None,
        &f.key.id,
        true,
        false,
    )
    .await
    .unwrap();
    f.mock.lock().await.callback_status = 202;
    f.event(&["lost-ack"]).await.unwrap();
    assert_eq!(
        f.mock.lock().await.callbacks.last().unwrap()["message_id"],
        before
    );
}

#[tokio::test]
async fn aurinko_inflight_claim_is_retryable_and_concurrent_batches_forward_once() {
    let f = Fixture::new().await;
    let EventDedupClaimResult::Claimed(claim) = EventDedupStore::claim(
        &f.state.db,
        "aurinko-inbound",
        &f.bot.id,
        "one",
        Duration::from_secs(60),
    )
    .await
    .unwrap() else {
        panic!()
    };
    assert!(f.event(&["one"]).await.is_err());
    assert!(f.mock.lock().await.callbacks.is_empty());
    EventDedupStore::release(&f.state.db, &claim).await.unwrap();
    let (a, b) = tokio::join!(f.event(&["one"]), f.event(&["one"]));
    assert!(a.is_ok() || b.is_ok());
    f.event(&["one"]).await.unwrap();
    assert_eq!(f.mock.lock().await.callbacks.len(), 1);
}

#[tokio::test]
async fn aurinko_reply_is_sender_only_and_incomplete_submission_is_never_retried() {
    let f = Fixture::new().await;
    f.event(&["one"]).await.unwrap();
    let original = f.original().await;
    assert!(
        f.reply(
            &original,
            Some(json!({"to":[{"address":"attacker@example.com"}]}))
        )
        .await
        .is_err()
    );
    assert!(matches!(
        f.adapter
            .send_bound_reply(
                &f.state.db,
                &f.state.http_client,
                &f.bot,
                &original,
                &BotCredentials {
                    billing: None,
                    token: TOKEN,
                    platform_bot_id: Some("42"),
                    platform_secrets: None,
                },
                original.platform_conversation_id.as_deref().unwrap(),
                &OutboundReply {
                    text: Some("Reply body".into()),
                    reply_to_platform_message_id: original.platform_message_id.clone(),
                    metadata: None,
                    attachments: vec![MaterializedAttachment {
                        kind: MediaKind::File,
                        bytes: bytes::Bytes::from_static(b"private file"),
                        filename: Some("private.txt".into()),
                        mime_type: None,
                        caption: None,
                    }],
                },
            )
            .await,
        Err(AppError::ValidationError(_))
    ));
    assert!(f.mock.lock().await.replies.is_empty());
    f.mock.lock().await.send_body = json!({"status":"Ok","processingStatus":"Incomplete","processingError":{"failedSteps":["returnIds"]}});
    let (a, b) = tokio::join!(f.reply(&original, None), f.reply(&original, None));
    assert!(a.is_ok() || b.is_ok());
    assert_eq!(f.reply(&original, None).await.unwrap(), None);
    let mock = f.mock.lock().await;
    assert_eq!(mock.replies.len(), 1);
    let reply = &mock.replies[0];
    assert_eq!(reply["to"], json!([{"address":"reply@example.com"}]));
    assert_eq!(reply["cc"], json!([]));
    assert_eq!(reply["bcc"], json!([]));
    let mut received = mail("automated");
    received["internetHeaders"] = reply["xHeaders"].clone();
    assert!(automated(&received));
    drop(mock);
    let row = f
        .state
        .db
        .collection::<bson::Document>(SENDS)
        .find_one(doc! {"bot_id":&f.bot.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.get_str("status").unwrap(), "submitted");
    assert!(!format!("{row:?}").contains("Reply body"));
}

#[tokio::test]
async fn aurinko_reply_preflight_retry_and_uncertain_post_barrier() {
    let f = Fixture::new().await;
    f.event(&["one"]).await.unwrap();
    let original = f.original().await;
    for status in [404, 408, 429, 503] {
        f.mock.lock().await.statuses.insert("one".into(), status);
        assert!(f.reply(&original, None).await.is_err());
    }
    assert_eq!(
        f.state
            .db
            .collection::<EmailSend>(SENDS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.mock.lock().await.statuses.clear();
    f.mock.lock().await.send_status = 500;
    assert!(f.reply(&original, None).await.is_err());
    assert!(f.reply(&original, None).await.is_err());
    assert_eq!(f.mock.lock().await.replies.len(), 1);
}

#[tokio::test]
async fn aurinko_registration_rotation_repair_delete_and_owner_fences() {
    let f = Fixture::new().await;
    let update = |token| bots::UpdateBotParams {
        bot_token: Some(token),
        label: None,
        verification_token: None,
        encrypt_key: bots::SecretPatch::Unchanged,
        app_id: None,
        app_secret: None,
    };
    assert!(
        bots::update_bot(
            &f.state.db,
            &f.state.encryption_keys,
            &f.state.http_client,
            &f.adapter,
            &f.bot.id,
            &f.bot.user_id,
            update("other-account")
        )
        .await
        .is_err()
    );
    assert!(
        bots::update_bot(
            &f.state.db,
            &f.state.encryption_keys,
            &f.state.http_client,
            &f.adapter,
            &f.bot.id,
            "foreign",
            update(TOKEN)
        )
        .await
        .is_err()
    );
    bots::update_bot(
        &f.state.db,
        &f.state.encryption_keys,
        &f.state.http_client,
        &f.adapter,
        &f.bot.id,
        &f.bot.user_id,
        update("rotated-token"),
    )
    .await
    .unwrap();
    let verified = bots::verify_serialized_bot(
        &f.state.db,
        &f.state.encryption_keys,
        &f.state.http_client,
        &f.adapter,
        &f.bot.id,
        &f.bot.user_id,
        &f.url,
    )
    .await
    .unwrap();
    assert_eq!(verified.status, "active");
    assert_eq!(f.mock.lock().await.subscriptions.len(), 1);
    // An unrelated subscription must survive deletion.
    f.mock.lock().await.subscriptions.push(json!({"id":999,"notificationUrl":"https://elsewhere.test/consumer","resource":"/email/messages","active":true}));
    assert_eq!(
        bots::delete_bot(
            &f.state.db,
            &f.state.config,
            &f.state.http_client,
            &f.state.encryption_keys,
            &f.adapter,
            &f.bot.id,
            &f.bot.user_id
        )
        .await
        .unwrap(),
        Some("removed")
    );
    assert_eq!(f.mock.lock().await.subscriptions.len(), 1);
    assert!(
        bots::verify_serialized_bot(
            &f.state.db,
            &f.state.encryption_keys,
            &f.state.http_client,
            &f.adapter,
            &f.bot.id,
            &f.bot.user_id,
            &f.url
        )
        .await
        .is_err()
    );
    assert!(f.event(&["one"]).await.is_err());
}

#[tokio::test]
async fn aurinko_failed_setup_delete_with_revoked_token_deactivates_locally() {
    let f = Fixture::new().await;
    let encrypted = f.state.encryption_keys.encrypt(b"revoked").await.unwrap();
    f.state.db.collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME).update_one(doc!{"_id":&f.bot.id},doc!{"$set":{"status":"failed","webhook_registered":false,"bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted}}}).await.unwrap();
    assert_eq!(
        bots::delete_bot(
            &f.state.db,
            &f.state.config,
            &f.state.http_client,
            &f.state.encryption_keys,
            &f.adapter,
            &f.bot.id,
            &f.bot.user_id
        )
        .await
        .unwrap(),
        Some("failed")
    );
    assert!(
        !bots::get_bot(&f.state.db, &f.bot.id)
            .await
            .unwrap()
            .is_active
    );
}

#[tokio::test]
async fn aurinko_repair_replaces_signal_subscription_and_recovers_unknown_creation() {
    let f = Fixture::new().await;
    f.mock.lock().await.subscriptions[0]["detailLevel"] = json!("signal");
    bots::verify_serialized_bot(
        &f.state.db,
        &f.state.encryption_keys,
        &f.state.http_client,
        &f.adapter,
        &f.bot.id,
        &f.bot.user_id,
        &f.url,
    )
    .await
    .unwrap();
    assert_eq!(f.mock.lock().await.subscriptions.len(), 1);
    assert_eq!(
        f.mock.lock().await.subscriptions[0]["detailLevel"],
        "status"
    );
    f.state
        .db
        .collection::<EmailSubscription>(SUBSCRIPTIONS)
        .update_one(
            doc! {"_id":&f.bot.id},
            doc! {"$set":{"subscription_id":bson::Bson::Null}},
        )
        .await
        .unwrap();
    bots::verify_serialized_bot(
        &f.state.db,
        &f.state.encryption_keys,
        &f.state.http_client,
        &f.adapter,
        &f.bot.id,
        &f.bot.user_id,
        &f.url,
    )
    .await
    .unwrap();
    assert_eq!(f.mock.lock().await.subscriptions.len(), 1);
}

#[tokio::test]
async fn aurinko_batch_timeout_cursor_allows_later_messages_progress() {
    let f = Fixture::new().await;
    {
        let mut mock = f.mock.lock().await;
        mock.delays.insert("slow1".into(), 11000);
        mock.delays.insert("slow2".into(), 11000);
    }
    assert!(f.event(&["slow1", "slow2", "healthy"]).await.is_err());
    assert!(f.event(&["slow1", "slow2", "healthy"]).await.is_err());
    assert_eq!(f.mock.lock().await.callbacks.len(), 1);
    assert_eq!(
        f.mock.lock().await.callbacks[0]["raw_platform_data"]["message_id"],
        "healthy"
    );
}

#[tokio::test]
async fn aurinko_concurrent_same_account_registration_has_one_owner() {
    let f = Fixture::new().await;
    bots::delete_bot(
        &f.state.db,
        &f.state.config,
        &f.state.http_client,
        &f.state.encryption_keys,
        &f.adapter,
        &f.bot.id,
        &f.bot.user_id,
    )
    .await
    .unwrap();
    let fields = RegistrationValues(BTreeMap::from([
        ("bot_token", TOKEN),
        ("app_secret", SECRET),
    ]));
    let owner2 = uuid::Uuid::new_v4().to_string();
    let create = |owner| {
        bots::create_bot(
            &f.state.db,
            &f.state.config,
            &f.state.encryption_keys,
            &f.state.http_client,
            &f.adapter,
            owner,
            "New mailbox",
            &fields,
        )
    };
    let (a, b) = tokio::join!(create(&f.bot.user_id), create(&owner2));
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(
        f.state
            .db
            .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
            .count_documents(doc! {"platform":"aurinko","is_active":true})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn aurinko_completion_survives_message_and_claim_expiry() {
    let f = Fixture::new().await;
    f.event(&["one"]).await.unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .delete_many(doc! {"channel_bot_id":&f.bot.id})
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::coordination::EVENT_DEDUP_COLLECTION_NAME)
        .delete_many(doc! {"namespace":"aurinko-inbound"})
        .await
        .unwrap();
    let body=serde_json::to_vec(&json!({"accountId":42,"subscription":100,"resource":"/email/messages","payloads":[{"id":"one","changeType":"updated"}]})).unwrap();
    f.adapter
        .receive(
            &f.context(),
            &f.bot.id,
            &signed(&body, Utc::now().timestamp()),
            &HashMap::new(),
            &body,
        )
        .await
        .unwrap();
    assert_eq!(f.mock.lock().await.callbacks.len(), 1);
}

#[tokio::test]
async fn aurinko_owner_deletion_removes_bot_secrets_and_cannot_recreate_ingress_state() {
    let f = Fixture::new().await;
    f.event(&["one"]).await.unwrap();
    let user = crate::test_utils::test_user(&f.bot.user_id, crate::models::user::UserType::Person);
    f.state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(&user)
        .await
        .unwrap();
    let legacy_id = uuid::Uuid::new_v4().to_string();
    let other_owner = uuid::Uuid::new_v4().to_string();
    for collection in [
        crate::models::channel_bot::COLLECTION_NAME,
        crate::models::channel_conversation::COLLECTION_NAME,
        crate::models::channel_message::COLLECTION_NAME,
    ] {
        f.state
            .db
            .collection::<bson::Document>(collection)
            .insert_many([
                doc! { "_id": &legacy_id, "user_id": &f.bot.user_id, "platform": "telegram" },
                doc! { "_id": &other_owner, "user_id": &other_owner, "platform": "aurinko" },
            ])
            .await
            .unwrap();
    }
    crate::services::admin_user_service::delete_current_user_cascade(&f.state.db, &f.bot.user_id)
        .await
        .unwrap();
    assert!(f.event(&["two"]).await.is_err());
    for collection in [
        crate::models::channel_bot::COLLECTION_NAME,
        crate::models::channel_email::SUBSCRIPTIONS,
        crate::models::channel_email::RECEIPTS,
        crate::models::channel_email::BATCHES,
        crate::models::channel_email::SENDS,
    ] {
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(collection)
                .count_documents(doc! {"user_id":&f.bot.user_id, "_id": { "$ne": &legacy_id }})
                .await
                .unwrap(),
            0
        );
    }
    for collection in [
        crate::models::channel_bot::COLLECTION_NAME,
        crate::models::channel_conversation::COLLECTION_NAME,
        crate::models::channel_message::COLLECTION_NAME,
    ] {
        let rows = f.state.db.collection::<bson::Document>(collection);
        assert_eq!(
            rows.count_documents(doc! { "user_id": &f.bot.user_id, "platform": "aurinko" })
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            rows.count_documents(doc! { "_id": &legacy_id })
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            rows.count_documents(doc! { "_id": &other_owner })
                .await
                .unwrap(),
            1
        );
    }
}

#[tokio::test]
async fn aurinko_rotated_signing_secret_requires_a_fresh_successful_challenge() {
    let f = Fixture::new().await;
    let patch = |secret| bots::UpdateBotParams {
        bot_token: None,
        label: None,
        verification_token: None,
        encrypt_key: bots::SecretPatch::Unchanged,
        app_id: None,
        app_secret: Some(secret),
    };
    let bot = bots::update_bot(
        &f.state.db,
        &f.state.encryption_keys,
        &f.state.http_client,
        &f.adapter,
        &f.bot.id,
        &f.bot.user_id,
        patch("wrong-secret"),
    )
    .await
    .unwrap();
    assert_eq!(bot.status, "pending_webhook");
    assert!(!bot.webhook_registered);
    assert!(
        bots::verify_serialized_bot(
            &f.state.db,
            &f.state.encryption_keys,
            &f.state.http_client,
            &f.adapter,
            &f.bot.id,
            &f.bot.user_id,
            &f.url
        )
        .await
        .is_err()
    );
    assert_ne!(
        bots::get_bot(&f.state.db, &f.bot.id).await.unwrap().status,
        "active"
    );
    bots::update_bot(
        &f.state.db,
        &f.state.encryption_keys,
        &f.state.http_client,
        &f.adapter,
        &f.bot.id,
        &f.bot.user_id,
        patch(SECRET),
    )
    .await
    .unwrap();
    assert_eq!(
        bots::verify_serialized_bot(
            &f.state.db,
            &f.state.encryption_keys,
            &f.state.http_client,
            &f.adapter,
            &f.bot.id,
            &f.bot.user_id,
            &f.url
        )
        .await
        .unwrap()
        .status,
        "active"
    );
}

#[tokio::test]
async fn aurinko_lifecycle_notifications_and_poison_messages_are_ignored() {
    let f = Fixture::new().await;
    let body=serde_json::to_vec(&json!({"accountId":42,"subscription":100,"resource":"/email/messages","lifecycleEvent":"error","error":"must-not-log"})).unwrap();
    f.adapter
        .receive(
            &f.context(),
            &f.bot.id,
            &signed(&body, Utc::now().timestamp()),
            &HashMap::new(),
            &body,
        )
        .await
        .unwrap();
    let mut oversized = mail("large");
    oversized["body"] = json!("x".repeat(MAX_BODY + 1));
    f.mock
        .lock()
        .await
        .messages
        .insert("large".into(), oversized);
    f.mock.lock().await.statuses.insert("deleted".into(), 404);
    f.event(&["large", "deleted", "healthy"]).await.unwrap();
    assert_eq!(f.mock.lock().await.callbacks.len(), 1);
}

#[tokio::test]
async fn aurinko_catalog_account_token_connection_owner_scope_and_disable() {
    use crate::models::{downstream_service::DownstreamService, provider_config::ProviderConfig};
    use crate::services::{
        catalog_spec_sync, mcp_service as mcp, provider_service, unified_key_service as keys,
        user_service_service,
    };
    let db = crate::test_utils::connect_test_database("aurinko_catalog")
        .await
        .expect("real MongoDB required");
    let state = crate::test_utils::test_app_state(db.clone());
    provider_service::seed_default_providers(&db, &state.encryption_keys)
        .await
        .unwrap();
    provider_service::seed_default_services(&db, &state.encryption_keys)
        .await
        .unwrap();
    catalog_spec_sync::sync_seeded_service_endpoints(&db)
        .await
        .unwrap();
    let provider = db
        .collection::<ProviderConfig>(crate::models::provider_config::COLLECTION_NAME)
        .find_one(doc! {"slug":"aurinko"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(provider.provider_type, "api_key");
    assert!(provider.authorization_url.is_none());
    assert!(provider.token_url.is_none());
    let providers = crate::services::platform_key_service::load_providers(&db)
        .await
        .unwrap();
    let catalog = db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .find_one(doc! {"slug":"api-aurinko"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(catalog.base_url, ORIGIN);
    assert!(catalog.requires_user_credential);
    let spec = crate::services::catalog_spec_registry::spec_for_slug("api-aurinko").unwrap();
    for (method, path, risk) in [
        (
            "POST",
            "/v1/email/sync",
            crate::models::service_endpoint::EndpointRisk::Write,
        ),
        (
            "GET",
            "/v1/email/sync/updated",
            crate::models::service_endpoint::EndpointRisk::Read,
        ),
        (
            "GET",
            "/v1/email/sync/deleted",
            crate::models::service_endpoint::EndpointRisk::Read,
        ),
    ] {
        let endpoint = db.collection::<crate::models::service_endpoint::ServiceEndpoint>(crate::models::service_endpoint::COLLECTION_NAME)
            .find_one(doc! { "service_id": &catalog.id, "method": method, "path": path, "is_active": true })
            .await.unwrap().expect("sync operation is seeded and discoverable");
        assert_eq!(endpoint.risk, Some(risk));
        assert!(!endpoint.supports_idempotency_key);
        let marker = &spec["paths"][path][method.to_ascii_lowercase()]["x-aevatar-tool"];
        assert_eq!(marker["readOnly"], method == "GET");
        assert_eq!(marker["requiresApproval"], method == "POST");
        assert_eq!(marker["destructive"], false);
    }
    let owner = uuid::Uuid::new_v4().to_string();
    let other = uuid::Uuid::new_v4().to_string();
    let created = keys::create_key(
        &db,
        &state.encryption_keys,
        &owner,
        &owner,
        Some("api-aurinko"),
        None,
        TOKEN,
        "My mailbox",
        None,
        None,
        None,
        None,
        None,
        None,
        keys::OpenApiSpecUrlInput::Inherit,
        None,
        false,
        keys::OauthClientCredentialsInput::None,
        false,
    )
    .await
    .unwrap();
    assert_eq!(created.endpoint.url, ORIGIN);
    assert_eq!(created.service.auth_method, "bearer");
    let credential = created.api_key.unwrap();
    assert_eq!(credential.user_id, owner);
    assert_ne!(
        credential.credential_encrypted.as_deref().unwrap(),
        TOKEN.as_bytes()
    );
    assert_eq!(
        state
            .encryption_keys
            .decrypt(credential.credential_encrypted.as_deref().unwrap())
            .await
            .unwrap(),
        TOKEN.as_bytes()
    );
    assert!(
        keys::list_keys(&db, &state.encryption_keys, &other, &providers)
            .await
            .unwrap()
            .is_empty()
    );
    let allowed = vec![created.service.id.clone()];
    let operations = mcp::load_operation_catalog(
        &db,
        &state.node_ws_manager,
        &owner,
        mcp::NodeScope::Unrestricted,
        mcp::ServiceScope::Allowed(&allowed),
    )
    .await
    .unwrap();
    assert_eq!(operations.services.len(), 1);
    assert_eq!(operations.services[0].endpoints.len(), 15);
    for (method, path) in [
        ("POST", "/v1/email/sync"),
        ("GET", "/v1/email/sync/updated"),
        ("GET", "/v1/email/sync/deleted"),
    ] {
        assert!(
            operations.services[0]
                .endpoints
                .iter()
                .any(|endpoint| endpoint.method == method && endpoint.path == path)
        );
    }
    assert!(!operations.services[0].is_generic_proxy);
    assert!(operations.services[0].executable);
    assert!(
        operations.services[0]
            .endpoints
            .iter()
            .any(|e| e.method == "POST" && e.path == "/v1/email/messages/{messageId}/reply")
    );
    assert!(
        mcp::load_operation_catalog(
            &db,
            &state.node_ws_manager,
            &other,
            mcp::NodeScope::Unrestricted,
            mcp::ServiceScope::Allowed(&allowed)
        )
        .await
        .unwrap()
        .services
        .is_empty()
    );
    assert!(
        mcp::load_operation_catalog(
            &db,
            &state.node_ws_manager,
            &owner,
            mcp::NodeScope::Unrestricted,
            mcp::ServiceScope::Allowed(&[])
        )
        .await
        .unwrap()
        .services
        .is_empty()
    );
    user_service_service::update_user_service(
        &db,
        &owner,
        &owner,
        &created.service.id,
        None,
        None,
        None,
        None,
        Some(false),
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(
        !keys::list_keys(&db, &state.encryption_keys, &owner, &providers)
            .await
            .unwrap()[0]
            .is_active
    );
    assert!(
        mcp::load_operation_catalog(
            &db,
            &state.node_ws_manager,
            &owner,
            mcp::NodeScope::Unrestricted,
            mcp::ServiceScope::Allowed(&allowed)
        )
        .await
        .unwrap()
        .services
        .is_empty()
    );
}

#[tokio::test]
async fn aurinko_owner_deletion_fences_delayed_ingress_and_reply() {
    for replying in [false, true] {
        let f = Fixture::new().await;
        let user =
            crate::test_utils::test_user(&f.bot.user_id, crate::models::user::UserType::Person);
        f.state
            .db
            .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(user)
            .await
            .unwrap();
        if replying {
            f.event(&["one"]).await.unwrap();
        }
        let original = if replying {
            Some(f.original().await)
        } else {
            None
        };
        f.mock.lock().await.delays.insert("one".into(), 400);
        f.mock.lock().await.requests.clear();
        let effect = async {
            match original.as_ref() {
                Some(original) => {
                    f.reply(original, None).await.unwrap();
                }
                None => {
                    f.event(&["one"]).await.unwrap();
                }
            }
        };
        let deletion = async {
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if f.mock
                        .lock()
                        .await
                        .requests
                        .iter()
                        .any(|(_, path)| path.starts_with("/v1/email/messages/one"))
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            // Busy effects make deletion retryable before deactivation.
            // The user must still be able to authenticate to retry.
            assert!(
                crate::services::admin_user_service::delete_current_user_cascade(
                    &f.state.db,
                    &f.bot.user_id
                )
                .await
                .is_err()
            );
            assert!(
                f.state
                    .db
                    .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
                    .find_one(doc! {"_id":&f.bot.user_id})
                    .await
                    .unwrap()
                    .unwrap()
                    .is_active
            );
        };
        tokio::join!(effect, deletion);
        crate::services::admin_user_service::delete_current_user_cascade(
            &f.state.db,
            &f.bot.user_id,
        )
        .await
        .unwrap();
        assert!(f.event(&["after-delete"]).await.is_err());
        assert_eq!(f.mock.lock().await.replies.len(), usize::from(replying));
        assert_eq!(f.mock.lock().await.callbacks.len(), 1);
        for collection in [
            SUBSCRIPTIONS,
            SENDS,
            crate::models::channel_email::BATCHES,
            crate::models::channel_email::RECEIPTS,
        ] {
            assert_eq!(
                f.state
                    .db
                    .collection::<bson::Document>(collection)
                    .count_documents(doc! {"user_id":&f.bot.user_id})
                    .await
                    .unwrap(),
                0
            );
        }
    }
}

#[tokio::test]
async fn aurinko_individual_delete_busy_ingress_keeps_bot_visible_until_retry() {
    let f = Fixture::new().await;
    f.mock.lock().await.delays.insert("one".into(), 400);
    f.mock.lock().await.requests.clear();
    let effect = f.event(&["one"]);
    let deletion = async {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if f.mock
                    .lock()
                    .await
                    .requests
                    .iter()
                    .any(|(_, path)| path.starts_with("/v1/email/messages/one"))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            bots::delete_bot(
                &f.state.db,
                &f.state.config,
                &f.state.http_client,
                &f.state.encryption_keys,
                &f.adapter,
                &f.bot.id,
                &f.bot.user_id
            )
            .await
            .is_err()
        );
        assert_eq!(
            bots::list_bots(&f.state.db, &f.bot.user_id)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(
            f.state
                .db
                .collection::<crate::models::channel_conversation::ChannelConversation>(
                    crate::models::channel_conversation::COLLECTION_NAME
                )
                .find_one(doc! {"_id":&f.route.id})
                .await
                .unwrap()
                .unwrap()
                .is_active
        );
    };
    let (delivered, ()) = tokio::join!(effect, deletion);
    delivered.unwrap();
    assert_eq!(
        bots::delete_bot(
            &f.state.db,
            &f.state.config,
            &f.state.http_client,
            &f.state.encryption_keys,
            &f.adapter,
            &f.bot.id,
            &f.bot.user_id
        )
        .await
        .unwrap(),
        Some("removed")
    );
    assert!(f.mock.lock().await.subscriptions.is_empty());
    assert!(
        bots::list_bots(&f.state.db, &f.bot.user_id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(f.event(&["two"]).await.is_err());
    assert_eq!(f.mock.lock().await.callbacks.len(), 1);
}

#[tokio::test]
async fn aurinko_late_thread_retries_and_invalid_ids_do_not_poison_batch() {
    let f = Fixture::new().await;
    let mut pending = mail("late");
    pending.as_object_mut().unwrap().remove("threadId");
    f.mock.lock().await.messages.insert("late".into(), pending);
    assert!(f.event(&["late", "..", "healthy"]).await.is_err());
    assert_eq!(f.mock.lock().await.callbacks.len(), 1);
    f.mock.lock().await.messages.remove("late");
    f.event(&["late", "..", "healthy"]).await.unwrap();
    assert_eq!(f.mock.lock().await.callbacks.len(), 2);
    assert_eq!(
        f.mock.lock().await.callbacks[1]["raw_platform_data"]["thread_id"],
        "thread-1"
    );
}

#[tokio::test]
async fn aurinko_route_revocation_during_preflight_prevents_send() {
    let f = Fixture::new().await;
    f.event(&["one"]).await.unwrap();
    let original = f.original().await;
    f.mock.lock().await.delays.insert("one".into(), 400);
    f.mock.lock().await.requests.clear();
    let revoke = async {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if f.mock
                    .lock()
                    .await
                    .requests
                    .iter()
                    .any(|(_, path)| path.starts_with("/v1/email/messages/one"))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        f.state
            .db
            .collection::<ApiKey>(crate::models::api_key::COLLECTION_NAME)
            .update_one(doc! {"_id":&f.key.id}, doc! {"$set":{"is_active":false}})
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(f.reply(&original, None), revoke);
    assert!(result.is_err());
    assert!(f.mock.lock().await.replies.is_empty());
    assert_eq!(
        f.state
            .db
            .collection::<EmailSend>(SENDS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[test]
fn aurinko_upstream_paths_cannot_escape_the_fixed_origin() {
    let adapter = AurinkoAdapter::default();
    let url = adapter
        .url(&["email", "messages", "https://evil.test/mail?a=b#c"])
        .unwrap();
    assert_eq!(url.origin().ascii_serialization(), ORIGIN);
    assert!(url.query().is_none());
    assert!(url.fragment().is_none());
    assert!(
        url.path()
            .contains("https:%2F%2Fevil.test%2Fmail%3Fa=b%23c")
    );
    for id in [".", "..", "", "line\nbreak"] {
        assert!(adapter.url(&["email", "messages", id]).is_err());
    }
}

#[tokio::test]
async fn aurinko_indexes_preserve_existing_platform_rows_and_are_repeatable() {
    let db = crate::test_utils::connect_test_database("aurinko_additive_indexes")
        .await
        .expect("real Mongo required");
    let bots = db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME);
    let existing: Vec<_> = ["telegram", "telegram", "slack", "whatsapp"].into_iter().map(|platform| doc! {
        "_id": uuid::Uuid::new_v4().to_string(), "user_id": uuid::Uuid::new_v4().to_string(),
        "platform": platform, "platform_bot_id": "42", "is_active": true,
    }).collect();
    bots.insert_many(existing.clone()).await.unwrap();
    for _ in 0..2 {
        ensure_indexes(&db).await.unwrap();
    }
    for row in existing {
        assert_eq!(
            bots.find_one(doc! { "_id": row.get_str("_id").unwrap() })
                .await
                .unwrap(),
            Some(row)
        );
    }
}
