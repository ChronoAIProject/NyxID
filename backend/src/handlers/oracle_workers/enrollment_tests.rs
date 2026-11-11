use std::net::SocketAddr;

use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::AppState;
use crate::models::{
    oracle_pool::{OraclePool, OraclePoolVisibility},
    org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole},
    user::{COLLECTION_NAME as USERS, User, UserType},
};
use crate::services::{oracle_pool_service, oracle_worker_enrollment_service as enrollment};
use crate::test_utils::{
    connect_transaction_test_database, test_app_state, test_membership, test_user,
};

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
        let db = connect_transaction_test_database("oracle_enrollment_http").await;
        enrollment::ensure_indexes(&db).await.unwrap();
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

    async fn org_pool(&self, member: &str, admin: &str) -> (OraclePool, String) {
        let org = Uuid::new_v4().to_string();
        self.state
            .db
            .collection::<User>(USERS)
            .insert_one(test_user(&org, UserType::Org))
            .await
            .unwrap();
        self.state
            .db
            .collection::<OrgMembership>(MEMBERSHIPS)
            .insert_many([
                test_membership(&org, member, OrgRole::Member, None),
                test_membership(&org, admin, OrgRole::Admin, None),
            ])
            .await
            .unwrap();
        oracle_pool_service::create_pool(
            &self.state.db,
            &org,
            oracle_pool_service::CreatePoolInput {
                slug: "contributed".into(),
                name: "Contributed".into(),
                visibility: Some(OraclePoolVisibility::Org),
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }
}

fn credential() -> String {
    format!("nyx_owi_{}", hex::encode(rand::random::<[u8; 32]>()))
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

/// Worker enrollment went with the rest of the built-in oracle: the manager
/// route mints no credential, and the worker routes honour neither a per-worker
/// credential nor the legacy pool token.
#[tokio::test]
async fn oracle_enrollment_http_answers_410_oracle_retired() {
    let server = Server::new().await;
    let (member, member_jwt) = server.human().await;
    let (admin, _) = server.human().await;
    let (pool, legacy) = server.org_pool(&member, &admin).await;
    let instance = Uuid::new_v4().to_string();
    let secret = credential();
    let enrolled = server
        .http
        .post(format!("{}/pools/{}/workers/enroll", server.url, pool.slug))
        .bearer_auth(&member_jwt)
        .json(&json!({ "installation_id": instance, "label": "mine", "credential": secret }))
        .send()
        .await
        .unwrap();
    assert_retired(enrolled).await;

    // Even a credential written straight through the service buys nothing.
    enrollment::enroll(
        &server.state.db,
        &member,
        &pool,
        &instance,
        Some("mine"),
        &secret,
    )
    .await
    .unwrap();
    let url = format!("{}/worker", server.url);
    let query = [("worker", "mine"), ("instance_id", instance.as_str())];
    for token in [&secret, &legacy] {
        let bundle = server
            .http
            .get(format!("{url}/bundle"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_retired(bundle).await;
        let task = server
            .http
            .get(format!("{url}/task"))
            .query(&query)
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_retired(task).await;
        let heartbeat = server
            .http
            .post(format!("{url}/heartbeat"))
            .bearer_auth(token)
            .json(&json!({ "worker": "mine", "instance_id": instance }))
            .send()
            .await
            .unwrap();
        assert_retired(heartbeat).await;
    }
}
