use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use bson::{Document, doc};
use serde_json::{Value, json};
use tower::ServiceExt;
use tracing::instrument::WithSubscriber;
use uuid::Uuid;

use crate::{
    crypto::jwt,
    models::{
        oauth_client::COLLECTION_NAME as CLIENTS,
        user::{COLLECTION_NAME as USERS, User, UserType},
    },
    services::{consent_service, oauth_client_service, role_service},
    test_utils::{connect_transaction_test_database, test_app_state, test_user},
};

const FLAG: &str = "auth:app-token-management-v1";

struct Fixture {
    db: mongodb::Database,
    app: Router,
    actor: Uuid,
    human: String,
    oauth: String,
    client: String,
}

impl Fixture {
    async fn new(name: &str) -> Self {
        let db = connect_transaction_test_database(name).await;
        let state = test_app_state(db.clone());
        let actor = Uuid::new_v4();
        let mut owner = test_user(&actor.to_string(), UserType::Person);
        role_service::seed_system_roles(&db).await.unwrap();
        owner.role_ids.push(
            role_service::get_platform_role_ids(&db)
                .await
                .unwrap()
                .admin,
        );
        db.collection::<User>(USERS)
            .insert_one(owner)
            .await
            .unwrap();
        let human = jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &actor,
            "openid",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let (client, _) = oauth_client_service::create_client(
            &db,
            "Management fixture",
            &["https://client.example/callback".into()],
            "public",
            &actor.to_string(),
            "",
            "openid",
            crate::models::oauth_client::ScopeProvenance::Explicit,
            false,
            None,
            None,
            &[],
        )
        .await
        .unwrap();
        let consent = consent_service::grant_consent(&db, &actor.to_string(), &client.id, "openid")
            .await
            .unwrap();
        let reference = consent_service::ensure_issuance_reference(&db, &consent)
            .await
            .unwrap();
        let oauth = jwt::generate_oauth_access_token_with_consent(
            &state.jwt_keys,
            &state.config,
            &actor,
            "openid",
            None,
            None,
            None,
            None,
            None,
            &client.id,
            Some(&reference),
        )
        .unwrap();
        let (public, private) = super::build_router_with_state(state.clone());
        Self {
            db,
            app: public.merge(private).with_state(state),
            actor,
            human,
            oauth,
            client: client.id,
        }
    }

    async fn request(
        &self,
        method: &str,
        path: &str,
        token: &str,
        data: Value,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if !token.is_empty() {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::from(data.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn designate(&self, trusted: bool, token: &str) -> (StatusCode, Value) {
        self.request(
            "PATCH",
            &format!("/api/v1/admin/oauth-clients/{}", self.client),
            token,
            json!({"first_party_management":trusted}),
        )
        .await
    }

    async fn toggle(&self, enabled: bool) {
        let (status, value) = self
            .request(
                "PUT",
                &format!("/api/v1/admin/feature-flags/{FLAG}"),
                &self.human,
                json!({"target_kind":"global","enabled":enabled}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
    }
}

#[tokio::test]
async fn app_management_shadow_logs_a_template_once_and_preserves_the_handler() {
    let f = Fixture::new("app_management_observe").await;
    let file = tempfile::NamedTempFile::new().unwrap();
    let writer = file.reopen().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.try_clone().unwrap())
        .finish();
    let missing_key = Uuid::new_v4();
    let path = format!("/api/v1/api-keys/{missing_key}?example=private-test-value");
    async {
        for _ in 0..4 {
            let (status, _) = f.request("GET", &path, &f.oauth, Value::Null).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
        }
    }
    .with_subscriber(subscriber)
    .await;
    let log = std::fs::read_to_string(file.path()).unwrap();
    let lines: Vec<_> = log
        .lines()
        .filter(|l| l.contains("app_token_management_refusal"))
        .collect();
    assert_eq!(lines.len(), 1);
    let line = lines[0];
    for expected in [&f.client, "GET", "/api/v1/api-keys/{key_id}", "count=1"] {
        assert!(line.contains(expected));
    }
    for private in [
        &f.oauth,
        &f.actor.to_string(),
        &missing_key.to_string(),
        "private-test-value",
    ] {
        assert!(!line.contains(private));
    }
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn app_management_rollout_rechecks_designation_and_preserves_read_contracts() {
    let f = Fixture::new("app_management_rollout").await;
    let protected = "/api/v1/api-keys";
    // Documents written by an earlier replica may lack the designation.
    f.db.collection::<Document>(CLIENTS)
        .update_one(
            doc! {"_id": &f.client},
            doc! {"$unset":{"first_party_management":""}},
        )
        .await
        .unwrap();
    assert_eq!(
        f.request("GET", protected, &f.oauth, Value::Null).await.0,
        StatusCode::OK
    );
    let now = bson::DateTime::now();
    f.db.collection::<Document>(crate::models::feature_flag_override::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": Uuid::new_v4().to_string(), "flag_key": FLAG,
            "target_kind": "global", "enabled": true, "updated_by": f.actor.to_string(),
            "created_at": now, "updated_at": now,
        })
        .await
        .unwrap();
    let (status, body) = f.request("GET", protected, &f.oauth, Value::Null).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error_code"], 1002);
    assert_eq!(body["error"], "forbidden");
    assert_eq!(
        body["message"],
        "Forbidden: A first-party human account session is required"
    );
    assert_eq!(body["details"]["accepted"], json!(["user_session"]));
    assert_eq!(body["details"]["credential_type"], "oauth_client");
    assert_eq!(body["details"]["reason"], "credential_type_unsupported");
    for path in [
        "/api/v1/orgs",
        "/api/v1/keys",
        "/api/v1/endpoints",
        "/api/v1/user-services",
        "/api/v1/api-keys/external",
        "/api/v1/catalog",
        "/api/v1/channel-bots",
        "/api/v1/channel-conversations",
        "/oauth/userinfo",
    ] {
        let (status, body) = f.request("GET", path, &f.oauth, Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
    }
    for (method, path) in [
        ("POST", "/api/v1/api-keys"),
        ("DELETE", "/api/v1/api-keys/fixture"),
        ("POST", "/api/v1/channel-bots"),
        ("PUT", "/api/v1/keys/fixture"),
        ("GET", "/api/v1/orgs/fixture/invites"),
    ] {
        let (status, body) = f.request(method, path, &f.oauth, Value::Null).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
        assert_eq!(body["details"]["credential_type"], "oauth_client");
    }
    assert_eq!(f.designate(true, &f.human).await.0, StatusCode::OK);
    assert_eq!(
        f.request("GET", protected, &f.oauth, Value::Null).await.0,
        StatusCode::OK
    );
    assert_eq!(
        f.request("GET", "/api/v1/billing/wallet/view", &f.oauth, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    // A designated owner reaches the same key and approval handlers as an account
    // session. Empty names and missing IDs keep these checks free of mutations.
    for (method, path, payload) in [
        ("POST", protected, json!({"name":""})),
        ("DELETE", "/api/v1/api-keys/fixture", Value::Null),
        (
            "POST",
            "/api/v1/approvals/requests/fixture/decide",
            Value::Null,
        ),
    ] {
        let human = f.request(method, path, &f.human, payload.clone()).await;
        let app = f.request(method, path, &f.oauth, payload).await;
        assert!(human.0.is_client_error() && human.0 != StatusCode::FORBIDDEN);
        assert_eq!(app, human);
    }
    assert_eq!(f.designate(false, &f.human).await.0, StatusCode::OK);
    assert_eq!(
        f.request("GET", protected, &f.oauth, Value::Null).await.0,
        StatusCode::FORBIDDEN
    );
    f.toggle(false).await;
    assert_eq!(
        f.request("GET", protected, &f.oauth, Value::Null).await.0,
        StatusCode::OK
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn app_management_designation_requires_a_platform_admin_session_and_records_its_value() {
    let f = Fixture::new("app_management_designation").await;
    let clients = f.db.collection::<Document>(CLIENTS);
    // Even with the rollout off, an app bearing the administrator's identity
    // cannot designate its own client.
    assert_eq!(f.designate(true, &f.oauth).await.0, StatusCode::FORBIDDEN);
    let (status, _) = f
        .request(
            "PATCH",
            &format!("/api/v1/developer/oauth-clients/{}", f.client),
            &f.human,
            json!({"name":"Renamed","first_party_management":true}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, registration) = f.request("POST","/oauth/register","",
        json!({"client_name":"fixture","redirect_uris":["https://client.example/cb"],"first_party_management":true})).await;
    assert_eq!(status, StatusCode::CREATED, "{registration}");
    for id in [
        f.client.as_str(),
        registration["client_id"].as_str().unwrap(),
    ] {
        let row = clients.find_one(doc! {"_id":id}).await.unwrap().unwrap();
        assert_ne!(row.get_bool("first_party_management").ok(), Some(true));
    }
    let (status, updated) = f.designate(true, &f.human).await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["first_party_management"], true);
    let rollout_path = format!("/api/v1/admin/feature-flags/{FLAG}");
    let (status, _) = f
        .request(
            "PUT",
            &rollout_path,
            &f.oauth,
            json!({"target_kind":"global","enabled":true}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = f
        .request(
            "DELETE",
            &format!("{rollout_path}?target_kind=global"),
            &f.oauth,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let audit = f.db.collection::<Document>(crate::models::security_effect_intent::COLLECTION_NAME)
        .find_one(doc!{"event.event_type":"admin_oauth_client_updated","event.event_data.client_id":&f.client})
        .await.unwrap().expect("committed audit intent");
    let event = audit.get_document("event").unwrap();
    assert_eq!(event.get_str("user_id").unwrap(), f.actor.to_string());
    let data = event.get_document("event_data").unwrap();
    assert!(data.get_bool("first_party_management").unwrap());
    assert!(
        data.get_array("changed_fields")
            .unwrap()
            .contains(&bson::Bson::from("first_party_management"))
    );
    f.db.collection::<User>(USERS)
        .update_one(
            doc! {"_id":f.actor.to_string()},
            doc! {"$set":{"role_ids":[],"is_admin":false}},
        )
        .await
        .unwrap();
    assert_eq!(f.designate(false, &f.human).await.0, StatusCode::FORBIDDEN);
    f.db.drop().await.unwrap();
}
