use std::net::SocketAddr;

use axum::http::StatusCode;
use base64::Engine;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::AppState;
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::services::{oracle_login_profile_service, oracle_pool_service, oracle_worker_service};
use crate::test_utils::{connect_transaction_test_database, test_app_state, test_user};

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
    async fn new() -> Self {
        let db = connect_transaction_test_database("oracle_login_profile_http").await;
        oracle_login_profile_service::ensure_indexes(&db)
            .await
            .unwrap();
        let state = test_app_state(db);
        let (_, private) = crate::routes::build_router();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/api/v1/oracle", listener.local_addr().unwrap());
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

    async fn worker(&self, pool: &crate::models::oracle_pool::OraclePool, instance: &str) {
        oracle_worker_service::allocate_worker(&self.state.db, pool, Some("worker"))
            .await
            .unwrap();
        oracle_worker_service::report_presence(
            &self.state.db,
            pool,
            oracle_worker_service::WorkerPresenceInput {
                worker_label: "worker".into(),
                instance_id: Some(instance.into()),
                capabilities: vec![oracle_login_profile_service::SAVED_LOGIN_CAPABILITY.into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn oracle_login_profile_http_guards_credentials_and_identity() {
    let server = Server::new().await;
    let (owner, owner_token) = server.human().await;
    let (stranger, stranger_token) = server.human().await;
    let (pool, worker_token) = oracle_pool_service::create_pool(
        &server.state.db,
        &owner,
        oracle_pool_service::CreatePoolInput {
            slug: "http-login".into(),
            name: "HTTP login".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let url = format!("{}/pools/{}/login-profiles", server.url, pool.id);
    let sealed = base64::engine::general_purpose::STANDARD.encode(b"opaque-session-fixture");
    let body = json!({ "format_version": 1, "worker_token_sha256": pool.worker_token_hash,
        "sealed_blob_base64": sealed, "expected_generation": null });
    let denied = server
        .http
        .put(format!("{url}/account"))
        .bearer_auth(&stranger_token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let saved = server
        .http
        .put(format!("{url}/account"))
        .bearer_auth(&owner_token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(saved.status(), StatusCode::OK);
    let saved: Value = saved.json().await.unwrap();
    assert!(saved.get("sealed_blob_base64").is_none());
    assert!(!saved.to_string().contains(&sealed));
    assert!(!saved.to_string().contains(&pool.worker_token_hash));
    let denied = server
        .http
        .get(&url)
        .bearer_auth(&stranger_token)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    let instance = Uuid::new_v4().to_string();
    server.worker(&pool, &instance).await;
    let query = [("worker", "worker"), ("instance_id", instance.as_str())];
    let worker_url = format!("{}/worker/login-profile", server.url);
    let unbound: Value = server
        .http
        .get(&worker_url)
        .query(&query)
        .bearer_auth(&worker_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(unbound["status"], "unbound");
    let binding_url = format!(
        "{}/pools/{}/workers/worker/login-profile",
        server.url, pool.id
    );
    let bound = server
        .http
        .put(&binding_url)
        .bearer_auth(&owner_token)
        .json(&json!({ "login_profile": "account" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bound.status(), StatusCode::OK);
    let current = server
        .http
        .get(&worker_url)
        .query(&query)
        .bearer_auth(&worker_token)
        .send()
        .await
        .unwrap();
    assert_eq!(current.status(), StatusCode::OK);
    let current: Value = current.json().await.unwrap();
    assert_eq!(current["sealed_blob_base64"], sealed);
    let unchanged: Value = server
        .http
        .get(&worker_url)
        .query(&query)
        .query(&[("known_revision", saved["revision"].as_str().unwrap())])
        .bearer_auth(&worker_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(unchanged.get("sealed_blob_base64").is_none());
    let wrong_instance = server
        .http
        .get(&worker_url)
        .query(&[
            ("worker", "worker"),
            ("instance_id", "another-installation"),
        ])
        .bearer_auth(&worker_token)
        .send()
        .await
        .unwrap();
    assert_eq!(wrong_instance.status(), StatusCode::CONFLICT);
    assert!(!wrong_instance.text().await.unwrap().contains(&sealed));
    let jwt = server
        .http
        .get(&worker_url)
        .query(&query)
        .bearer_auth(&owner_token)
        .send()
        .await
        .unwrap();
    assert_eq!(jwt.status(), StatusCode::UNAUTHORIZED);

    let (other_pool, other_token) = oracle_pool_service::create_pool(
        &server.state.db,
        &stranger,
        oracle_pool_service::CreatePoolInput {
            slug: "other-pool".into(),
            name: "Other".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    server.worker(&other_pool, &instance).await;
    let other: Value = server
        .http
        .get(&worker_url)
        .query(&query)
        .bearer_auth(&other_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(other["status"], "unbound");
    let cross_pool = server
        .http
        .post(&worker_url)
        .bearer_auth(&other_token)
        .json(&json!({
            "worker": "worker", "instance_id": instance, "profile_id": saved["id"],
            "binding_id": current["binding"]["binding_id"], "generation": saved["generation"],
            "expected_revision": saved["revision"], "publication_id": Uuid::new_v4().to_string(),
            "format_version": 1, "sealed_blob_base64": sealed,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(cross_pool.status(), StatusCode::CONFLICT);

    let (_, rotated_token) =
        oracle_pool_service::rotate_worker_token(&server.state.db, &owner, &pool.id)
            .await
            .unwrap();
    let old = server
        .http
        .get(&worker_url)
        .query(&query)
        .bearer_auth(&worker_token)
        .send()
        .await
        .unwrap();
    assert_eq!(old.status(), StatusCode::UNAUTHORIZED);
    let rotated: Value = server
        .http
        .get(&worker_url)
        .query(&query)
        .bearer_auth(&rotated_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(rotated["status"], "token_changed");
    assert!(rotated.get("sealed_blob_base64").is_none());
}

#[tokio::test]
async fn oracle_login_profile_http_caps_both_credential_ingresses() {
    let server = Server::new().await;
    let (owner, owner_token) = server.human().await;
    let (pool, worker_token) = oracle_pool_service::create_pool(
        &server.state.db,
        &owner,
        oracle_pool_service::CreatePoolInput {
            slug: "capped-login".into(),
            name: "Capped".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let oversized = json!({ "sealed_blob_base64": "A".repeat(
        crate::services::oracle_login_snapshot_service::MAX_LOGIN_SNAPSHOT_BASE64_CHARS + 8192) });
    let manager = server
        .http
        .put(format!(
            "{}/pools/{}/login-profiles/account",
            server.url, pool.id
        ))
        .bearer_auth(&owner_token)
        .json(&oversized)
        .send()
        .await
        .unwrap();
    assert_eq!(manager.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let worker = server
        .http
        .post(format!("{}/worker/login-profile", server.url))
        .bearer_auth(&worker_token)
        .json(&oversized)
        .send()
        .await
        .unwrap();
    assert_eq!(worker.status(), StatusCode::PAYLOAD_TOO_LARGE);
}
