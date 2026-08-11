use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::AppResult;
use crate::mw::auth::AuthUser;
use crate::services::assistant_action_execution_service::{
    self, KeyCreateActionRequest, KeyCreateActionResult,
};

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateAssistantKeyRequest {
    pub action_request_id: String,
    pub name: String,
    pub platform: String,
    pub allowed_service_ids: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantKeyResource {
    pub key_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateAssistantKeyResponse {
    pub resource: AssistantKeyResource,
    pub replayed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_key: Option<String>,
}

/// Create the exact least-scope agent key requested by a browser action.
/// Replays return only the durable safe resource identity.
pub async fn create_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateAssistantKeyRequest>,
) -> AppResult<Json<CreateAssistantKeyResponse>> {
    auth_user.ensure_write_scope()?;
    let result = assistant_action_execution_service::create_key(
        &state.db,
        &auth_user.user_id.to_string(),
        KeyCreateActionRequest {
            action_request_id: body.action_request_id,
            name: body.name,
            platform: body.platform,
            allowed_service_ids: body.allowed_service_ids,
        },
    )
    .await?;

    Ok(Json(match result {
        KeyCreateActionResult::Created(created) => CreateAssistantKeyResponse {
            resource: AssistantKeyResource { key_id: created.id },
            replayed: false,
            full_key: Some(created.full_key),
        },
        KeyCreateActionResult::Replayed { key_id } => CreateAssistantKeyResponse {
            resource: AssistantKeyResource { key_id },
            replayed: true,
            full_key: None,
        },
    }))
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode},
        routing::{get, post},
    };
    use mongodb::{IndexModel, bson::doc, options::IndexOptions};
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::handlers::api_keys;
    use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
    use crate::models::assistant_action_receipt::COLLECTION_NAME as ASSISTANT_ACTION_RECEIPTS;
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
    use crate::test_utils::{connect_test_database, test_app_state, test_user, test_user_service};

    fn access_token(state: &AppState, user_id: &str) -> String {
        crate::crypto::jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &Uuid::parse_str(user_id).expect("valid user id"),
            "",
            None,
            None,
            None,
            None,
            None,
        )
        .expect("sign test access token")
    }

    fn app(state: AppState) -> Router {
        Router::new()
            .route("/assistant/actions/key-create", post(create_key))
            .route("/api-keys/{key_id}", get(api_keys::get_key))
            .with_state(state)
    }

    async fn request(
        app: Router,
        token: &str,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"));
        let body = match body {
            Some(value) => {
                builder = builder.header("content-type", "application/json");
                Body::from(value.to_string())
            }
            None => Body::empty(),
        };
        let response = app
            .oneshot(builder.body(body).expect("build request"))
            .await
            .expect("route response");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response");
        let value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(&bytes).into_owned() }));
        (status, value)
    }

    async fn prepare_database(
        prefix: &str,
    ) -> Option<(mongodb::Database, String, UserService, UserService)> {
        let db = connect_test_database(prefix).await?;
        db.collection::<mongodb::bson::Document>(ASSISTANT_ACTION_RECEIPTS)
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "user_id": 1, "action": 1, "action_request_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await
            .expect("create receipt uniqueness index");

        let actor_id = Uuid::new_v4().to_string();
        let other_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&actor_id, UserType::Person),
                test_user(&other_id, UserType::Person),
            ])
            .await
            .expect("insert users");

        let own_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &actor_id,
            "own-service",
            &Uuid::new_v4().to_string(),
            None,
            None,
        );
        let other_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &other_id,
            "other-service",
            &Uuid::new_v4().to_string(),
            None,
            None,
        );
        db.collection::<UserService>(USER_SERVICES)
            .insert_many([own_service.clone(), other_service.clone()])
            .await
            .expect("insert services");

        Some((db, actor_id, own_service, other_service))
    }

    fn create_body(action_request_id: &str, service_ids: Value) -> Value {
        json!({
            "actionRequestId": action_request_id,
            "name": "coding-agent",
            "platform": "codex",
            "allowedServiceIds": service_ids,
        })
    }

    #[tokio::test]
    async fn key_create_fails_closed_for_adversarial_service_sets() {
        let Some((db, actor_id, own_service, other_service)) =
            prepare_database("assistant_key_create_adversarial").await
        else {
            return;
        };
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);
        let unknown_id = Uuid::new_v4().to_string();
        let fixtures = [
            (
                "missing",
                json!({
                    "actionRequestId": "missing",
                    "name": "coding-agent",
                    "platform": "codex",
                }),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                "empty",
                create_body("empty", json!([])),
                StatusCode::BAD_REQUEST,
            ),
            (
                "duplicate",
                create_body(
                    "duplicate",
                    json!([own_service.id.clone(), own_service.id.clone()]),
                ),
                StatusCode::BAD_REQUEST,
            ),
            (
                "malformed",
                create_body("malformed", json!(["not/a/service-id"])),
                StatusCode::BAD_REQUEST,
            ),
            (
                "unknown",
                create_body("unknown", json!([unknown_id])),
                StatusCode::BAD_REQUEST,
            ),
            (
                "cross-owner",
                create_body("cross-owner", json!([other_service.id])),
                StatusCode::BAD_REQUEST,
            ),
            (
                "unknown-field",
                json!({
                    "actionRequestId": "unknown-field",
                    "name": "coding-agent",
                    "platform": "codex",
                    "allowedServiceIds": [own_service.id],
                    "allowAllServices": true,
                }),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
        ];

        for (label, body, expected_status) in fixtures {
            let (status, response) = request(
                app(state.clone()),
                &token,
                "POST",
                "/assistant/actions/key-create",
                Some(body),
            )
            .await;
            assert_eq!(status, expected_status, "{label}: {response}");
        }

        assert_eq!(
            db.collection::<ApiKey>(API_KEYS)
                .count_documents(doc! { "user_id": &actor_id })
                .await
                .expect("count keys"),
            0
        );
    }

    #[tokio::test]
    async fn exact_read_back_proves_least_scope_without_secret_material() {
        let Some((db, actor_id, own_service, _)) =
            prepare_database("assistant_key_create_read_back").await
        else {
            return;
        };
        let state = test_app_state(db);
        let token = access_token(&state, &actor_id);
        let body = create_body("read-back", json!([own_service.id.clone()]));

        let (status, created) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/key-create",
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{created}");
        assert_eq!(created["replayed"], false);
        let key_id = created["resource"]["keyId"]
            .as_str()
            .expect("safe key identity");
        assert!(created["fullKey"].as_str().is_some());

        let (status, read_back) = request(
            app(state.clone()),
            &token,
            "GET",
            &format!("/api-keys/{key_id}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{read_back}");
        assert_eq!(read_back["id"], key_id);
        assert_eq!(read_back["scopes"], "proxy");
        assert_eq!(read_back["allowed_service_ids"], json!([own_service.id]));
        assert_eq!(read_back["allow_all_services"], false);
        assert_eq!(read_back["allowed_node_ids"], json!([]));
        assert_eq!(read_back["allow_all_nodes"], false);
        for forbidden in ["fullKey", "full_key", "keyHash", "key_hash", "secret"] {
            assert!(
                read_back.get(forbidden).is_none(),
                "leaked field: {forbidden}"
            );
        }

        let (status, replayed) = request(
            app(state),
            &token,
            "POST",
            "/assistant/actions/key-create",
            Some(create_body("read-back", json!([own_service.id]))),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{replayed}");
        assert_eq!(replayed["resource"]["keyId"], key_id);
        assert_eq!(replayed["replayed"], true);
        assert!(replayed.get("fullKey").is_none());
    }

    #[test]
    fn replay_response_is_secret_free() {
        let response = CreateAssistantKeyResponse {
            resource: AssistantKeyResource {
                key_id: "key-alpha".to_string(),
            },
            replayed: true,
            full_key: None,
        };
        assert_eq!(
            serde_json::to_value(response).expect("serialize response"),
            json!({
                "resource": { "keyId": "key-alpha" },
                "replayed": true,
            })
        );
    }
}
