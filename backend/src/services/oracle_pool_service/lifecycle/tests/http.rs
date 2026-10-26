use super::*;
use axum::http::StatusCode;
use serde_json::{Value, json};
use std::net::SocketAddr;

struct Server {
    f: Fixture,
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
        let f = fixture("oracle_lifecycle_http").await;
        let state = crate::test_utils::test_app_state(f.db.clone());
        let (_, router) = crate::routes::build_router();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/api/v1/oracle/pools/{}",
            listener.local_addr().unwrap(),
            f.pool.slug
        );
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                router
                    .with_state(state)
                    .into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        Self {
            f,
            url,
            http: reqwest::Client::new(),
            task,
        }
    }
    fn jwt(&self, id: &str) -> String {
        let state = crate::test_utils::test_app_state(self.f.db.clone());
        crate::crypto::jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &uuid::Uuid::parse_str(id).unwrap(),
            "read write proxy",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap()
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

/// Pool lifecycle over HTTP went with the rest of the built-in oracle: the
/// owner can neither delete nor transfer the pool, and the pool is untouched.
/// The service-level lifecycle tests beside this file still cover the rules
/// themselves.
#[tokio::test]
async fn oracle_lifecycle_http_answers_410_oracle_retired() {
    let s = Server::new().await;
    let owner = s.jwt(&s.f.actor.user_id);
    let deleted = s
        .http
        .delete(&s.url)
        .bearer_auth(&owner)
        .send()
        .await
        .unwrap();
    assert_retired(deleted).await;
    let transferred = s
        .http
        .post(format!("{}/transfer", s.url))
        .bearer_auth(&owner)
        .json(&json!({"owner_user_id":s.f.org}))
        .send()
        .await
        .unwrap();
    assert_retired(transferred).await;
    assert!(get_pool(&s.f.db, &s.f.pool.id).await.is_ok());
}
