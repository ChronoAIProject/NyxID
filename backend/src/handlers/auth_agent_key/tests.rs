use super::*;
use crate::models::{
    api_key::{ApiKey, COLLECTION_NAME as API_KEYS},
    user::{COLLECTION_NAME as USERS, User, UserType},
};
use crate::test_utils::{connect_transaction_test_database, test_app_state, test_user};
use axum::http::StatusCode;
use mongodb::bson::doc;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::models::{
    user_api_key::{COLLECTION_NAME as EXTERNAL_KEYS, UserApiKey},
    user_endpoint::{COLLECTION_NAME as ENDPOINTS, UserEndpoint},
    user_service::{COLLECTION_NAME as SERVICES, UserService},
};
use crate::test_utils::{test_user_endpoint, test_user_service};

struct Server {
    state: AppState,
    url: String,
    http: reqwest::Client,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn new(name: &str) -> Self {
        let db = connect_transaction_test_database(name).await;
        crate::db::ensure_indexes(&db).await.unwrap();
        let state = test_app_state(db);
        let (_, private) = crate::routes::build_router();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = private.with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        Self {
            state,
            url,
            http: reqwest::Client::new(),
            task,
        }
    }
    async fn human(&self) -> (String, String) {
        let id = Uuid::new_v4();
        self.state
            .db
            .collection::<User>(USERS)
            .insert_one(test_user(&id.to_string(), UserType::Person))
            .await
            .unwrap();
        let token = crate::crypto::jwt::generate_access_token(
            &self.state.jwt_keys,
            &self.state.config,
            &id,
            "read write proxy",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        (id.to_string(), token)
    }
    async fn post(&self, endpoint: &str, token: Option<&str>, body: Value) -> (StatusCode, Value) {
        let mut request = self
            .http
            .post(format!("{}/api/v1/auth/agent-key/{endpoint}", self.url))
            .header(header::USER_AGENT, format!("AgentKeyTest/{endpoint}"))
            .json(&body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.unwrap();
        let status = response.status();
        let json = response.json().await.unwrap();
        (status, json)
    }
}

#[tokio::test]
async fn login_audit_records_capture_actor_context_and_request_identity() {
    use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDITS};
    let server = Server::new("akl_audit_context").await;
    let (actor, human) = server.human().await;
    for approve in [true, false] {
        let (_, request) = server.post("request", None, json!({})).await;
        let row = server
            .state
            .db
            .collection::<crate::models::agent_key_login_request::AgentKeyLoginRequest>(
                crate::models::agent_key_login_request::COLLECTION_NAME,
            )
            .find_one(doc! {"status": "pending"})
            .await
            .unwrap()
            .unwrap();
        let endpoint = if approve { "approve" } else { "deny" };
        let (status, body) = server.post(endpoint, Some(&human), json!({"user_code": request["user_code"], "selection": {"kind": "new", "name": "Audited", "scopes": "read proxy"}})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let mut delivery = Value::Null;
        if approve {
            let (status, value) = server
                .post("poll", None, json!({"device_code": request["device_code"]}))
                .await;
            assert_eq!(status, StatusCode::OK, "{value}");
            delivery = value;
        }
        let events: &[(&str, &str)] = if approve {
            &[
                ("agent_key_login_approved", "approve"),
                ("agent_key_login_delivered", "poll"),
            ]
        } else {
            &[("agent_key_login_denied", "deny")]
        };
        for (event, endpoint) in events {
            let audit = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    if let Some(audit) = server
                        .state
                        .db
                        .collection::<AuditLog>(AUDITS)
                        .find_one(doc! {"event_type": event, "event_data.request_id": &row.id})
                        .await
                        .unwrap()
                    {
                        break audit;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("audit append");
            assert_eq!(audit.user_id.as_deref(), Some(actor.as_str()));
            assert_eq!(audit.ip_address.as_deref(), Some("127.0.0.1"));
            assert_eq!(audit.user_agent, Some(format!("AgentKeyTest/{endpoint}")));
            if approve {
                let details = audit.event_data.as_ref().unwrap();
                assert_eq!(details["api_key_id"], delivery["api_key"]["id"]);
                assert_eq!(details["credential_id"], delivery["credential_id"]);
                assert!(
                    !serde_json::to_string(&audit)
                        .unwrap()
                        .contains(delivery["credential"].as_str().unwrap())
                );
            }
            let json = serde_json::to_string(&audit).unwrap();
            assert!(!json.contains(request["user_code"].as_str().unwrap()));
            assert!(!json.contains(request["device_code"].as_str().unwrap()));
        }
    }
}

#[derive(Clone)]
struct TraceCapture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
impl std::io::Write for TraceCapture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn traces_record_hashed_ip_identifiers_and_outcomes_without_secrets() {
    use tracing::instrument::WithSubscriber;
    let server = Server::new("akl_observability").await;
    let (actor, _) = server.human().await;
    let state = server.state.clone();
    let capture = TraceCapture(Default::default());
    let writer = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let (user_code, device_code, delivery) = async {
        let addr = "203.0.113.9:1234".parse().unwrap();
        let (_, Json(created)) = request(
            State(state.clone()),
            ConnectInfo(addr),
            HeaderMap::new(),
            Json(serde_json::from_value(json!({"client_label": "private-client-label"})).unwrap()),
        )
        .await
        .unwrap();
        for (code, expected) in [
            ("private-invalid-device-code", 11900),
            (created.request.device_code.as_str(), 11902),
            (created.request.device_code.as_str(), 11903),
        ] {
            let error = poll(
                State(state.clone()),
                ConnectInfo(addr),
                HeaderMap::new(),
                TelemetryContext::default(),
                Json(PollBody {
                    device_code: code.into(),
                }),
            )
            .await
            .unwrap_err();
            assert_eq!(error.error_code(), expected);
        }
        service::approve(
            &state.db,
            &state.encryption_keys,
            state.auth_device_hmac_key.as_slice(),
            &actor,
            &created.request.user_code,
            serde_json::from_value(
                json!({"kind": "new", "name": "Trace key", "scopes": "read proxy"}),
            )
            .unwrap(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let (_, Json(delivery)) = poll(
            State(state.clone()),
            ConnectInfo(addr),
            HeaderMap::new(),
            TelemetryContext::default(),
            Json(PollBody {
                device_code: created.request.device_code.clone(),
            }),
        )
        .await
        .unwrap();
        for _ in 0..4 {
            let _ = request(
                State(state.clone()),
                ConnectInfo(addr),
                HeaderMap::new(),
                Json(serde_json::from_value(json!({})).unwrap()),
            )
            .await
            .unwrap();
        }
        assert!(matches!(
            request(
                State(state.clone()),
                ConnectInfo(addr),
                HeaderMap::new(),
                Json(serde_json::from_value(json!({})).unwrap())
            )
            .await,
            Err(AppError::AgentKeyLoginRateLimited)
        ));
        (
            created.request.user_code,
            created.request.device_code,
            delivery,
        )
    }
    .with_subscriber(subscriber)
    .await;
    let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    for (i, expected) in [
        "client_ip_hash",
        "row_id",
        "requested",
        "pending",
        "slow_down",
        "not_found",
        "delivered",
        "rate_limit_hit",
        "agent_key_login.poll.outcome",
        &delivery.api_key.id,
        &delivery.credential_id,
    ]
    .into_iter()
    .enumerate()
    {
        assert!(logs.contains(expected), "missing marker #{i}");
    }
    let ip_hash = crate::services::auth_device_service::hmac_hex(
        state.auth_device_hmac_key.as_slice(),
        b"203.0.113.9",
    );
    assert!(logs.contains(&ip_hash));
    for secret in [
        &user_code,
        &device_code,
        &delivery.credential,
        "203.0.113.9",
        "private-client-label",
        "private-invalid-device-code",
    ] {
        assert!(!logs.contains(secret), "unexpected secret in tracing");
    }
}

#[tokio::test]
async fn enrolled_credential_executes_only_allowed_proxy_with_live_parent_bindings_and_scopes() {
    let server = Server::new("akl_proxy_http").await;
    let (actor, human) = server.human().await;
    let db = &server.state.db;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let downstream_url = format!("http://{}", listener.local_addr().unwrap());
    let downstream = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/status",
                axum::routing::get(|headers: HeaderMap| async move {
                    headers
                        .get(header::AUTHORIZATION)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("none")
                        .to_string()
                }),
            ),
        )
        .await
        .unwrap();
    });
    let mut services = Vec::new();
    for slug in ["allowed", "denied"] {
        let endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &actor,
            slug,
            &downstream_url,
            None,
            None,
        );
        let service = test_user_service(
            &Uuid::new_v4().to_string(),
            &actor,
            slug,
            &endpoint.id,
            None,
            None,
        );
        db.collection::<UserEndpoint>(ENDPOINTS)
            .insert_one(endpoint)
            .await
            .unwrap();
        db.collection::<UserService>(SERVICES)
            .insert_one(&service)
            .await
            .unwrap();
        services.push(service);
    }
    let (_, request) = server.post("request", None, json!({})).await;
    let (status, approved) = server
        .post(
            "approve",
            Some(&human),
            json!({
                "user_code": request["user_code"], "selection": {
                    "kind": "new", "name": "Restricted proxy", "scopes": "read proxy",
                    "allowed_service_ids": [&services[0].id],
                },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    let (status, delivery) = server
        .post("poll", None, json!({"device_code": request["device_code"]}))
        .await;
    assert_eq!(status, StatusCode::OK, "{delivery}");
    let secret = delivery["credential"].as_str().unwrap();
    let key_id = delivery["api_key"]["id"].as_str().unwrap();
    let allowed_url = format!("{}/api/v1/proxy/s/allowed/status", server.url);
    let response = server
        .http
        .get(&allowed_url)
        .bearer_auth(secret)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-nyxid-agent-id"], key_id);
    assert_eq!(response.text().await.unwrap(), "none");
    let denied = server
        .http
        .get(format!("{}/api/v1/proxy/s/denied/status", server.url))
        .bearer_auth(secret)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_eq!(denied.json::<Value>().await.unwrap()["error_code"], 9000);

    let external_id = Uuid::new_v4().to_string();
    let encrypted = server
        .state
        .encryption_keys
        .encrypt(b"bound-provider-secret")
        .await
        .unwrap();
    let external: UserApiKey = mongodb::bson::from_document(doc! {
        "_id": &external_id, "user_id": &actor, "label": "Bound provider",
        "credential_type": "bearer", "status": "active",
        "credential_encrypted": mongodb::bson::Binary { subtype: mongodb::bson::spec::BinarySubtype::Generic, bytes: encrypted },
        "created_at": mongodb::bson::DateTime::now(), "updated_at": mongodb::bson::DateTime::now(),
    }).unwrap();
    let mut default_external = external.clone();
    default_external.id = Uuid::new_v4().to_string();
    default_external.credential_encrypted = Some(
        server
            .state
            .encryption_keys
            .encrypt(b"default-provider-secret")
            .await
            .unwrap(),
    );
    let default_id = default_external.id.clone();
    db.collection::<UserApiKey>(EXTERNAL_KEYS)
        .insert_many([external, default_external])
        .await
        .unwrap();
    db.collection::<UserService>(SERVICES)
        .update_one(
            doc! {"_id": &services[0].id},
            doc! {"$set": {"auth_method": "bearer", "api_key_id": default_id}},
        )
        .await
        .unwrap();
    crate::services::agent_binding_service::create_binding(
        db,
        &actor,
        key_id,
        &services[0].id,
        &external_id,
    )
    .await
    .unwrap();
    let response = server
        .http
        .get(&allowed_url)
        .bearer_auth(secret)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.text().await.unwrap(),
        "Bearer bound-provider-secret"
    );

    db.collection::<ApiKey>(API_KEYS)
        .update_one(doc! {"_id": key_id}, doc! {"$set": {"scopes": "read"}})
        .await
        .unwrap();
    assert_eq!(
        server
            .http
            .get(&allowed_url)
            .bearer_auth(secret)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    crate::services::key_service::delete_api_key(db, &actor, key_id)
        .await
        .unwrap();
    assert_eq!(
        server
            .http
            .get(&allowed_url)
            .bearer_auth(secret)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let logs = db.collection::<mongodb::bson::Document>("audit_log");
    let revoked = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(row) = logs.find_one(doc! {"event_type": "agent_key_credential_revoked", "event_data.credential_id": &delivery["credential_id"].as_str().unwrap()}).await.unwrap() {
                break row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }).await.unwrap();
    assert_eq!(
        revoked
            .get_document("event_data")
            .unwrap()
            .get_str("reason")
            .unwrap(),
        "parent_revoked"
    );
    assert!(!format!("{revoked:?}").contains(secret));
    assert!(
        logs.find_one(doc! {"event_type": "proxy_request", "api_key_id": key_id})
            .await
            .unwrap()
            .is_some()
    );
    downstream.abort();
}

#[tokio::test]
async fn public_request_preview_and_poll_need_no_account_and_never_return_browser_secrets() {
    let server = Server::new("akl_public_http").await;
    let (status, request) = server
        .post(
            "request",
            None,
            json!({"client_label": "requester", "requested_profile": "home-agent"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        request["verification_uri"]
            .as_str()
            .unwrap()
            .ends_with("/login/agent-key")
    );
    assert!(!request.to_string().contains("user_code="));
    let (status, preview) = server
        .post("preview", None, json!({"user_code": request["user_code"]}))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(preview["requested_profile"], "home-agent");
    assert_eq!(preview["status"], "pending");
    assert!(preview.get("api_key").is_none());
    assert!(preview.get("credential").is_none());
    assert!(preview.get("device_code").is_none());
    let (_, pending) = server
        .post("poll", None, json!({"device_code": request["device_code"]}))
        .await;
    assert_eq!(pending["error_code"], 11902);
    let (_, slow) = server
        .post("poll", None, json!({"device_code": request["device_code"]}))
        .await;
    assert_eq!(slow["error_code"], 11903);
    for _ in 0..4 {
        server.post("request", None, json!({})).await;
    }
    let (status, limited) = server.post("request", None, json!({})).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(limited["error_code"], 11906);
}

#[tokio::test]
async fn decisions_reject_primary_child_service_account_delegated_and_relay_credentials() {
    let server = Server::new("akl_human_http").await;
    let (_, human) = server.human().await;
    let mut forbidden = vec![
        "nyxid_ag_forbidden".to_string(),
        "nyxid_primary_forbidden".to_string(),
    ];
    for flag in ["sa", "delegated", "relay"] {
        let claims =
            crate::crypto::jwt::verify_token(&server.state.jwt_keys, &server.state.config, &human)
                .unwrap();
        let mut payload = serde_json::to_value(claims).unwrap();
        payload[flag] = json!(true);
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some(server.state.jwt_keys.kid.clone());
        forbidden.push(
            jsonwebtoken::encode(&header, &payload, &server.state.jwt_keys.encoding).unwrap(),
        );
    }
    for endpoint in ["options", "approve", "deny"] {
        for token in &forbidden {
            let (status, body) = server.post(endpoint, Some(token), json!({"user_code": "ABCD-EFGH", "selection": {"kind": "existing", "api_key_id": "not-called"}})).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{endpoint}: {body}");
        }
        let response = server
            .http
            .post(format!("{}/api/v1/auth/agent-key/{endpoint}", server.url))
            .header("X-API-Key", "nyxid_ag_forbidden")
            .json(&json!({"user_code": "ABCD-EFGH"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}

#[tokio::test]
async fn web_and_phone_choices_deliver_child_auth_and_support_scoped_logout_and_web_revoke() {
    let server = Server::new("akl_journeys_http").await;
    let (actor, human) = server.human().await;
    let (_, outsider) = server.human().await;
    let mut key_id = String::new();
    for create in [true, false] {
        let (_, request) = server
            .post(
                "request",
                None,
                json!({"client_label": "workstation", "requested_profile": "home-agent"}),
            )
            .await;
        let code = &request["user_code"];
        let (status, choices) = server
            .post("options", Some(&human), json!({"user_code": code}))
            .await;
        assert_eq!(status, StatusCode::OK, "{choices}");
        if !create {
            assert_eq!(choices["keys"][0]["id"], key_id);
        }
        let selection = if create {
            json!({"kind": "new", "name": "CLI key", "scopes": "read proxy"})
        } else {
            json!({"kind": "existing", "api_key_id": key_id})
        };
        let (status, approved) = server
            .post(
                "approve",
                Some(&human),
                json!({"user_code": code, "selection": selection}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{approved}");
        assert_eq!(approved, json!({"ok": true}));
        let (_, preview) = server
            .post("preview", None, json!({"user_code": code}))
            .await;
        assert_eq!(preview["status"], "approved");
        assert!(preview.get("api_key").is_none());
        assert!(preview.get("credential").is_none());
        let (status, delivery) = server
            .post("poll", None, json!({"device_code": request["device_code"]}))
            .await;
        assert_eq!(status, StatusCode::OK, "{delivery}");
        assert_eq!(delivery["label"], "workstation \u{00b7} home-agent");
        let (_, preview) = server
            .post("preview", None, json!({"user_code": code}))
            .await;
        assert_eq!(preview["status"], "delivered");
        assert!(preview.get("api_key").is_none());
        let secret = delivery["credential"].as_str().unwrap();
        key_id = delivery["api_key"]["id"].as_str().unwrap().to_string();
        assert_eq!(delivery["api_key"]["owner_id"], actor);
        assert_eq!(delivery["api_key"]["created_now"], create);
        assert_eq!(delivery["api_key"]["allow_all_services"], false);
        assert_eq!(delivery["api_key"]["allow_all_nodes"], false);
        assert!(delivery.get("refresh_token").is_none());
        for name in ["Authorization", "X-API-Key"] {
            let response = server
                .http
                .get(format!("{}/api/v1/auth/agent-key/self", server.url))
                .header(
                    name,
                    if name == "Authorization" {
                        format!("Bearer {secret}")
                    } else {
                        secret.to_string()
                    },
                )
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["cache-control"], "no-store");
            assert!(response.headers().get("set-cookie").is_none());
            let body: Value = response.json().await.unwrap();
            assert_eq!(body["credential_id"], delivery["credential_id"]);
            assert_eq!(body["label"], delivery["label"]);
            assert!(!body.to_string().contains(secret));
        }
        let list_url = format!("{}/api/v1/api-keys/{key_id}/credentials", server.url);
        let response = server
            .http
            .get(&list_url)
            .bearer_auth(&human)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let credentials: Value = response.json().await.unwrap();
        assert!(!credentials.to_string().contains(secret));
        assert_eq!(
            server
                .http
                .get(&list_url)
                .bearer_auth(&outsider)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        let revoke_url = format!(
            "{}/{}",
            list_url,
            delivery["credential_id"].as_str().unwrap()
        );
        assert_eq!(
            server
                .http
                .delete(&revoke_url)
                .bearer_auth(&outsider)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        let response = if create {
            server
                .http
                .delete(format!("{}/api/v1/auth/agent-key/self", server.url))
                .bearer_auth(secret)
                .send()
                .await
                .unwrap()
        } else {
            server
                .http
                .delete(&revoke_url)
                .bearer_auth(&human)
                .send()
                .await
                .unwrap()
        };
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            server
                .http
                .get(format!("{}/api/v1/auth/agent-key/self", server.url))
                .bearer_auth(secret)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert!(
            server
                .state
                .db
                .collection::<ApiKey>(API_KEYS)
                .find_one(doc! {"_id": &key_id, "is_active": true})
                .await
                .unwrap()
                .is_some()
        );
    }
    assert_eq!(
        server
            .http
            .delete(format!("{}/api/v1/auth/agent-key/self", server.url))
            .bearer_auth(&human)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
}
