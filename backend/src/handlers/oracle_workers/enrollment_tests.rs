use std::net::SocketAddr;

use axum::http::StatusCode;
use bson::doc;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::AppState;
use crate::models::{
    oracle_pool::{OraclePool, OraclePoolVisibility},
    org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole},
    user::{COLLECTION_NAME as USERS, User, UserType},
};
use crate::services::{
    oracle_pool_service, oracle_worker_enrollment_service as enrollment, oracle_worker_service,
};
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

#[tokio::test]
async fn oracle_enrollment_http_has_no_secret_response_and_scopes_worker_management() {
    let server = Server::new().await;
    let (member, member_jwt) = server.human().await;
    let (admin, admin_jwt) = server.human().await;
    let (outsider, outsider_jwt) = server.human().await;
    let (pool, legacy) = server.org_pool(&member, &admin).await;
    let pool_url = format!("{}/pools/{}", server.url, pool.slug);
    let instance = Uuid::new_v4().to_string();
    let secret = credential();
    let enrollment_body =
        json!({ "installation_id": instance, "label": "mine", "credential": secret });
    let denied = server
        .http
        .post(format!("{pool_url}/workers/enroll"))
        .bearer_auth(&outsider_jwt)
        .json(&enrollment_body)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_eq!(denied.headers()["cache-control"], "no-store");
    let enrolled = server
        .http
        .post(format!("{pool_url}/workers/enroll"))
        .bearer_auth(&member_jwt)
        .json(&enrollment_body)
        .send()
        .await
        .unwrap();
    assert_eq!(enrolled.status(), StatusCode::OK);
    assert_eq!(enrolled.headers()["cache-control"], "no-store");
    let payload: Value = enrolled.json().await.unwrap();
    assert_eq!(
        payload,
        json!({ "pool_id": pool.id, "pool_slug": pool.slug, "label": "mine", "installation_id": instance, "credential_type": "installation" })
    );
    assert!(!payload.to_string().contains(&secret));
    assert!(!payload.to_string().contains(&legacy));
    assert!(
        !format!(
            "{:?}",
            serde_json::from_value::<super::EnrollOracleWorkerRequest>(enrollment_body.clone())
                .unwrap()
        )
        .contains(&secret)
    );
    let meta: Value = server
        .http
        .get(&pool_url)
        .bearer_auth(&member_jwt)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(meta["can_enroll"], true);
    assert_eq!(meta["can_manage"], false);
    let metadata_list: Value = server
        .http
        .get(format!("{}/pools", server.url))
        .bearer_auth(&member_jwt)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(metadata_list["pools"][0]["can_enroll"], true);
    let sibling_secret = credential();
    let sibling_instance = Uuid::new_v4().to_string();
    enrollment::enroll(
        &server.state.db,
        &admin,
        &pool,
        &sibling_instance,
        Some("sibling"),
        &sibling_secret,
    )
    .await
    .unwrap();
    let heartbeat = server.http.post(format!("{}/worker/heartbeat", server.url)).bearer_auth(&secret)
        .json(&json!({"worker":"mine", "instance_id":instance,"capabilities":["commands_v1"], "logged_in":true})).send().await.unwrap();
    assert_eq!(heartbeat.status(), StatusCode::OK);
    let own_rows: Value = server
        .http
        .get(format!("{pool_url}/workers"))
        .bearer_auth(&member_jwt)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(own_rows["workers"].as_array().unwrap().len(), 1);
    assert_eq!(own_rows["workers"][0]["owner_user_id"], member);
    assert_eq!(own_rows["workers"][0]["credential_type"], "installation");
    assert_eq!(own_rows["workers"][0]["can_manage"], true);
    let all_rows: Value = server
        .http
        .get(format!("{pool_url}/workers"))
        .bearer_auth(&admin_jwt)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(all_rows["workers"].as_array().unwrap().len(), 2);
    assert_eq!(
        server
            .http
            .get(format!("{pool_url}/workers/sibling"))
            .bearer_auth(&member_jwt)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        server
            .http
            .get(format!("{pool_url}/workers/mine"))
            .bearer_auth(&member_jwt)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let queued = server
        .http
        .post(format!("{pool_url}/workers/mine/commands"))
        .bearer_auth(&member_jwt)
        .json(&json!({"command":"drain"}))
        .send()
        .await
        .unwrap();
    assert_eq!(queued.status(), StatusCode::ACCEPTED);
    let command: Value = queued.json().await.unwrap();
    assert_eq!(
        server
            .http
            .get(format!("{pool_url}/workers/mine/commands"))
            .bearer_auth(&member_jwt)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        server
            .http
            .delete(format!(
                "{pool_url}/workers/mine/commands/{}",
                command["id"].as_str().unwrap()
            ))
            .bearer_auth(&member_jwt)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    for path in [
        "workers/sibling/commands",
        "workers/allocate",
        "rotate-token",
    ] {
        assert_eq!(
            server
                .http
                .post(format!("{pool_url}/{path}"))
                .bearer_auth(&member_jwt)
                .json(&json!({"command":"drain"}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
    assert_eq!(
        server
            .http
            .patch(&pool_url)
            .bearer_auth(&member_jwt)
            .json(&json!({"name":"unauthorized"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        server
            .http
            .get(format!("{pool_url}/login-profiles"))
            .bearer_auth(&member_jwt)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        server
            .http
            .delete(format!("{pool_url}/workers/sibling?force=true"))
            .bearer_auth(&member_jwt)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        server
            .http
            .delete(format!("{pool_url}/workers/mine?force=true"))
            .bearer_auth(&member_jwt)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert!(
        enrollment::authenticate(&server.state.db, &secret)
            .await
            .is_err()
    );
    assert!(
        enrollment::authenticate(&server.state.db, &sibling_secret)
            .await
            .is_ok()
    );
    // Viewers see the pool but cannot contribute, and public visibility alone grants no enrollment.
    server
        .state
        .db
        .collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(test_membership(
            &pool.user_id,
            &outsider,
            OrgRole::Viewer,
            None,
        ))
        .await
        .unwrap();
    let viewer_meta: Value = server
        .http
        .get(&pool_url)
        .bearer_auth(&outsider_jwt)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(viewer_meta["can_enroll"], false);
    assert_eq!(
        server
            .http
            .post(format!("{pool_url}/workers/enroll"))
            .bearer_auth(&outsider_jwt)
            .json(&enrollment_body)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn oracle_enrollment_http_binds_every_worker_route_and_rejects_retired_credentials() {
    let server = Server::new().await;
    let (member, _) = server.human().await;
    let (admin, _) = server.human().await;
    let (pool, legacy) = server.org_pool(&member, &admin).await;
    let instance = Uuid::new_v4().to_string();
    let secret = credential();
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
    let valid = [("worker", "mine"), ("instance_id", instance.as_str())];
    assert_eq!(
        server
            .http
            .get(format!("{url}/bundle"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        server
            .http
            .get(format!("{url}/task"))
            .bearer_auth(&secret)
            .query(&valid)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        server
            .http
            .get(format!("{url}/bundle"))
            .bearer_auth(&legacy)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    for (label, installation) in [("sibling", instance.as_str()), ("mine", "different")] {
        let query = [("worker", label), ("instance_id", installation)];
        for path in ["task", "login-profile"] {
            assert_eq!(
                server
                    .http
                    .get(format!("{url}/{path}"))
                    .bearer_auth(&secret)
                    .query(&query)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNAUTHORIZED,
                "{path}"
            );
        }
        let body = json!({ "worker":label, "instance_id":installation, "task_id":"task", "response":"text",
            "chatgpt_url":"https://chatgpt.com/c/test", "turns":[], "profile_id":"profile", "binding_id":"binding",
            "generation":"generation", "expected_revision":"revision", "publication_id":"publication", "format_version":1, "sealed_blob_base64":"e30=" });
        for path in [
            "heartbeat",
            "ack",
            "result",
            "transcript",
            "worker/transcript",
            "pin-conv-url",
            "login-profile",
        ] {
            assert_eq!(
                server
                    .http
                    .post(format!("{url}/{path}"))
                    .bearer_auth(&secret)
                    .json(&body)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNAUTHORIZED,
                "{path}"
            );
        }
    }
    assert_eq!(
        server
            .http
            .get(format!("{url}/task"))
            .bearer_auth(&secret)
            .query(&[("worker", "mine")])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        server
            .http
            .get(format!("{url}/login-snapshots/any"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let current: Value = server
        .http
        .get(format!("{url}/login-profile"))
        .bearer_auth(&secret)
        .query(&valid)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(current, json!({"status":"unbound"}));
    server
        .state
        .db
        .collection::<bson::Document>(MEMBERSHIPS)
        .update_one(
            doc! { "member_user_id": &member },
            doc! { "$set": { "revoked_at": bson::DateTime::now() } },
        )
        .await
        .unwrap();
    assert_eq!(
        server
            .http
            .get(format!("{url}/bundle"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        server
            .http
            .get(format!("{url}/login-snapshots/any"))
            .bearer_auth(&secret)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        server
            .http
            .get(format!("{url}/task"))
            .bearer_auth(&secret)
            .query(&valid)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        oracle_worker_service::list_workers(&server.state.db, &pool.id)
            .await
            .unwrap()
            .len(),
        1
    );
}
