use std::net::SocketAddr;

use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::AppState;
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::services::oracle_pool_service;
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
}

async fn assert_retired(response: reqwest::Response) {
    assert_eq!(response.status(), StatusCode::GONE);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"], "oracle_retired");
    assert_eq!(body["error_code"], 11099);
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("permanently deprecated"),
        "{body}"
    );
}

/// The login-profile ingress went with the rest of the built-in oracle: the
/// manager route takes no credential blob from the pool owner, and the worker
/// route neither hands one out nor accepts one, whichever token presents it.
#[tokio::test]
async fn oracle_login_profile_http_answers_410_oracle_retired() {
    let server = Server::new().await;
    let (owner, owner_token) = server.human().await;
    let (pool, worker_token) = oracle_pool_service::create_pool(
        &server.state.db,
        &owner,
        oracle_pool_service::CreatePoolInput {
            slug: "retired-login".into(),
            name: "Retired login".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let body = json!({ "format_version": 1, "worker_token_sha256": pool.worker_token_hash,
        "sealed_blob_base64": "b3BhcXVlLXNlc3Npb24tZml4dHVyZQ==", "expected_generation": null });
    let manager = server
        .http
        .put(format!(
            "{}/pools/{}/login-profiles/account",
            server.url, pool.id
        ))
        .bearer_auth(&owner_token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_retired(manager).await;

    let worker_url = format!("{}/worker/login-profile", server.url);
    let query = [("worker", "worker"), ("instance_id", "installation-1")];
    let fetched = server
        .http
        .get(&worker_url)
        .query(&query)
        .bearer_auth(&worker_token)
        .send()
        .await
        .unwrap();
    assert_retired(fetched).await;
    let published = server
        .http
        .post(&worker_url)
        .bearer_auth(&worker_token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_retired(published).await;
}
