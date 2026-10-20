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
    async fn person(&self) -> (String, String) {
        let id = uuid::Uuid::new_v4().to_string();
        self.f
            .db
            .collection::<User>(USERS)
            .insert_one(test_user(&id, UserType::Person))
            .await
            .unwrap();
        let jwt = self.jwt(&id);
        (id, jwt)
    }
    async fn key(&self, owner: &str) -> String {
        let key = format!("nyxid_ag_{}", hex::encode(rand::random::<[u8; 32]>()));
        self.f.db.collection::<Document>(crate::models::api_key::COLLECTION_NAME).insert_one(doc! {
            "_id":uuid::Uuid::new_v4().to_string(),"user_id":owner,"name":"lifecycle general key",
            "key_prefix":&key[..16],"key_hash":hash_token(&key),"scopes":"read write proxy",
            "is_active":true,"created_at":bson::DateTime::now(),"purpose":"general"
        }).await.unwrap();
        key
    }
    async fn transfer(&self, token: &str, body: Value) -> reqwest::Response {
        self.http
            .post(format!("{}/transfer", self.url))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn oracle_lifecycle_http_auth_acl_full_response_and_secrets() {
    let s = Server::new().await;
    let unauthorized = s.http.delete(&s.url).send().await.unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let unauthorized = s
        .http
        .post(format!("{}/transfer", s.url))
        .json(&json!({"owner_user_id":s.f.org}))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let (_, stranger) = s.person().await;
    assert_eq!(
        s.http
            .delete(&s.url)
            .bearer_auth(&stranger)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let denied = s
        .transfer(&stranger, json!({"owner_user_id":s.f.org}))
        .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_eq!(denied.headers()["cache-control"], "no-store");
    let owner = s.jwt(&s.f.actor.user_id);
    for body in [
        json!({"owner_user_id":"not-uuid"}),
        json!({"owner_user_id":s.f.org,"unknown":true}),
    ] {
        assert!(s.transfer(&owner, body).await.status().is_client_error());
    }
    let large = s
        .transfer(&owner, json!({"owner_user_id":"x".repeat(5000)}))
        .await;
    assert_eq!(large.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(large.headers()["cache-control"], "no-store");
    let personal_key = s.key(&s.f.actor.user_id).await;
    let response = s
        .transfer(
            &personal_key,
            json!({"owner_user_id":s.f.org,"rotate_worker_token":true}),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["pool"]["id"], s.f.pool.id);
    assert_eq!(body["pool"]["owner_user_id"], s.f.org);
    assert_eq!(body["pool"]["can_manage"], true);
    assert_eq!(body["pool"]["can_enroll"], true);
    assert!(body["pool"].get("worker_token_hash").is_none());
    assert!(
        validate_worker_token(&s.f.db, body["worker_token"].as_str().unwrap())
            .await
            .is_ok()
    );
    assert!(validate_worker_token(&s.f.db, &s.f.token).await.is_err());
    // General org-owned keys retain Direct-owner management compatibility.
    let org_key = s.key(&s.f.org).await;
    let result = s
        .transfer(
            &org_key,
            json!({"owner_user_id":s.f.org,"rotate_worker_token":true}),
        )
        .await;
    assert_eq!(result.status(), StatusCode::OK);
    let body: Value = result.json().await.unwrap();
    assert_eq!(body["pool"]["can_enroll"], false);
    for role in [OrgRole::Member, OrgRole::Viewer] {
        let (person, jwt) = s.person().await;
        s.f.db
            .collection::<OrgMembership>(MEMBERSHIPS)
            .insert_one(test_membership(&s.f.org, &person, role, None))
            .await
            .unwrap();
        assert_eq!(
            s.http
                .delete(&s.url)
                .bearer_auth(&jwt)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            s.transfer(&jwt, json!({"owner_user_id":s.f.org}))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    let first = s
        .http
        .delete(&s.url)
        .bearer_auth(&org_key)
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let first = first.text().await.unwrap();
    let second = s
        .http
        .delete(&s.url)
        .bearer_auth(&org_key)
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(first, second.text().await.unwrap());
    let ack: Value = serde_json::from_str(&first).unwrap();
    assert_eq!(ack["status"], "deleted");
    assert_eq!(ack["cleanup"], "asynchronous");
}

#[tokio::test]
async fn oracle_lifecycle_http_typed_conflict_and_disabled_authority() {
    let s = Server::new().await;
    let owner = s.jwt(&s.f.actor.user_id);
    let task = tasks::submit_task(&s.f.db, &s.f.pool, &identity(&s.f), prompt())
        .await
        .unwrap()
        .task;
    let response = s
        .http
        .delete(&s.url)
        .bearer_auth(&owner)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error_code"], 11017);
    tasks::cancel_task(&s.f.db, &s.f.actor.user_id, &task.id, 30)
        .await
        .unwrap();
    s.f.db
        .collection::<Document>(USERS)
        .update_one(doc! {"_id":&s.f.org}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    assert_eq!(
        s.transfer(&owner, json!({"owner_user_id":s.f.org}))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    s.f.db
        .collection::<Document>(USERS)
        .update_one(
            doc! {"_id":&s.f.actor.user_id},
            doc! {"$set":{"is_active":false}},
        )
        .await
        .unwrap();
    assert!(
        s.http
            .delete(&s.url)
            .bearer_auth(&owner)
            .send()
            .await
            .unwrap()
            .status()
            .is_client_error()
    );
    assert!(get_pool(&s.f.db, &s.f.pool.id).await.is_ok());
}
