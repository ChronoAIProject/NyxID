use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::AppResult;
use crate::mw::auth::AuthUser;
use crate::services::assistant_action_execution_service::{
    self, KeyCreateActionRequest, KeyCreateActionResult, KeyRotateActionRequest,
    KeyRotateActionResult,
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

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RotateAssistantKeyRequest {
    pub action_request_id: String,
    pub key_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RotateAssistantKeyResponse {
    pub resource: AssistantKeyResource,
    pub replayed: bool,
    pub requested_at: String,
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

/// Rotate one exact owner-scoped key. The replacement secret is returned only
/// to the browser call that commits the reserved successor.
pub async fn rotate_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<RotateAssistantKeyRequest>,
) -> AppResult<Json<RotateAssistantKeyResponse>> {
    auth_user.ensure_write_scope()?;
    let result = assistant_action_execution_service::rotate_key(
        &state.db,
        &auth_user.user_id.to_string(),
        KeyRotateActionRequest {
            action_request_id: body.action_request_id,
            key_id: body.key_id,
        },
    )
    .await?;

    Ok(Json(match result {
        KeyRotateActionResult::Created {
            created,
            requested_at,
        } => RotateAssistantKeyResponse {
            resource: AssistantKeyResource { key_id: created.id },
            replayed: false,
            requested_at: requested_at.to_rfc3339(),
            full_key: Some(created.full_key),
        },
        KeyRotateActionResult::Replayed {
            key_id,
            requested_at,
        } => RotateAssistantKeyResponse {
            resource: AssistantKeyResource { key_id },
            replayed: true,
            requested_at: requested_at.to_rfc3339(),
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
    use crate::models::api_key::{ApiKey, ApiKeyPurpose, COLLECTION_NAME as API_KEYS};
    use crate::models::assistant_action_receipt::COLLECTION_NAME as ASSISTANT_ACTION_RECEIPTS;
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
    use crate::test_utils::{
        connect_test_database, connect_transaction_test_database, test_app_state, test_user,
        test_user_service,
    };

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
            .route("/assistant/actions/key-rotate", post(rotate_key))
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

    fn test_api_key(id: &str, user_id: &str, name: &str) -> ApiKey {
        let now = chrono::Utc::now() - chrono::Duration::minutes(5);
        ApiKey {
            id: id.to_string(),
            user_id: user_id.to_string(),
            name: name.to_string(),
            key_prefix: "safe-prefix".to_string(),
            key_hash: "deadbeef".repeat(8),
            scopes: "proxy".to_string(),
            last_used_at: None,
            expires_at: None,
            is_active: true,
            created_at: now,
            rotation_predecessor_id: None,
            state_version: 1,
            updated_at: Some(now),
            description: Some("assistant rotation fixture".to_string()),
            allowed_service_ids: Vec::new(),
            allowed_node_ids: Vec::new(),
            allow_all_services: true,
            allow_all_nodes: true,
            rate_limit_per_second: Some(10),
            rate_limit_burst: Some(20),
            platform: Some("codex".to_string()),
            callback_url: None,
            purpose: ApiKeyPurpose::General,
            scheduled_write_enabled: false,
        }
    }

    async fn prepare_rotation_database(
        prefix: &str,
    ) -> (mongodb::Database, String, String, String) {
        let db = connect_transaction_test_database(prefix).await;
        db.collection::<mongodb::bson::Document>(ASSISTANT_ACTION_RECEIPTS)
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "user_id": 1, "action": 1, "action_request_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await
            .expect("create receipt uniqueness index");
        db.collection::<mongodb::bson::Document>(API_KEYS)
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "rotation_predecessor_id": 1 })
                    .options(
                        IndexOptions::builder()
                            .unique(true)
                            .partial_filter_expression(
                                doc! { "rotation_predecessor_id": { "$type": "string" } },
                            )
                            .build(),
                    )
                    .build(),
            )
            .await
            .expect("create rotation lineage index");

        let actor_id = Uuid::new_v4().to_string();
        let other_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&actor_id, UserType::Person),
                test_user(&other_id, UserType::Person),
            ])
            .await
            .expect("insert users");
        let predecessor_id = Uuid::new_v4().to_string();
        let other_predecessor_id = Uuid::new_v4().to_string();
        db.collection::<ApiKey>(API_KEYS)
            .insert_many([
                test_api_key(&predecessor_id, &actor_id, "actor-key"),
                test_api_key(&other_predecessor_id, &other_id, "other-key"),
            ])
            .await
            .expect("insert predecessor keys");
        (db, actor_id, predecessor_id, other_predecessor_id)
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

        let rotation = RotateAssistantKeyResponse {
            resource: AssistantKeyResource {
                key_id: "key-replacement".to_string(),
            },
            replayed: true,
            requested_at: "2026-08-11T08:00:00Z".to_string(),
            full_key: None,
        };
        assert_eq!(
            serde_json::to_value(rotation).expect("serialize rotation response"),
            json!({
                "resource": { "keyId": "key-replacement" },
                "replayed": true,
                "requestedAt": "2026-08-11T08:00:00Z",
            })
        );
    }

    #[tokio::test]
    async fn exact_rotation_read_back_is_replay_safe_and_secret_free() {
        let (db, actor_id, predecessor_id, _) =
            prepare_rotation_database("assistant_key_rotate_exact").await;
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);
        let body = json!({
            "actionRequestId": "rotate-exact",
            "keyId": &predecessor_id,
        });

        let (status, rotated) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/key-rotate",
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{rotated}");
        assert_eq!(rotated["replayed"], false);
        assert!(rotated["fullKey"].as_str().is_some());
        let successor_id = rotated["resource"]["keyId"]
            .as_str()
            .expect("replacement key id");
        assert_ne!(successor_id, predecessor_id);
        let requested_at = chrono::DateTime::parse_from_rfc3339(
            rotated["requestedAt"].as_str().expect("requested at"),
        )
        .expect("valid requested timestamp");

        let (status, read_back) = request(
            app(state.clone()),
            &token,
            "GET",
            &format!("/api-keys/{successor_id}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{read_back}");
        assert_eq!(read_back["id"], successor_id);
        assert_eq!(read_back["rotation_predecessor_id"], predecessor_id);
        assert!(
            read_back["state_version"]
                .as_i64()
                .is_some_and(|value| value > 0)
        );
        for field in ["created_at", "updated_at"] {
            let timestamp = chrono::DateTime::parse_from_rfc3339(
                read_back[field].as_str().expect("authoritative timestamp"),
            )
            .expect("valid timestamp");
            assert!(timestamp >= requested_at, "{field} predates action request");
        }
        for forbidden in [
            "fullKey",
            "full_key",
            "keyHash",
            "key_hash",
            "secret",
            "authorization",
        ] {
            assert!(
                read_back.get(forbidden).is_none(),
                "leaked field: {forbidden}"
            );
        }

        let (status, replayed) = request(
            app(state),
            &token,
            "POST",
            "/assistant/actions/key-rotate",
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{replayed}");
        assert_eq!(replayed["resource"]["keyId"], successor_id);
        assert_eq!(replayed["requestedAt"], rotated["requestedAt"]);
        assert_eq!(replayed["replayed"], true);
        assert!(replayed.get("fullKey").is_none());
        assert_eq!(
            db.collection::<ApiKey>(API_KEYS)
                .count_documents(doc! { "user_id": &actor_id })
                .await
                .expect("count actor keys"),
            2
        );
    }

    #[tokio::test]
    async fn rotation_fails_closed_for_identity_conflicts_and_cross_owner_keys() {
        let (db, actor_id, predecessor_id, other_predecessor_id) =
            prepare_rotation_database("assistant_key_rotate_conflicts").await;
        let state = test_app_state(db.clone());
        let token = access_token(&state, &actor_id);

        let (status, first) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/key-rotate",
            Some(json!({
                "actionRequestId": "rotate-conflict",
                "keyId": &predecessor_id,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{first}");

        let local_second_id = Uuid::new_v4().to_string();
        db.collection::<ApiKey>(API_KEYS)
            .insert_one(test_api_key(&local_second_id, &actor_id, "second-key"))
            .await
            .expect("insert second local predecessor");
        let (status, conflict) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/key-rotate",
            Some(json!({
                "actionRequestId": "rotate-conflict",
                "keyId": &local_second_id,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{conflict}");

        let (status, hidden) = request(
            app(state.clone()),
            &token,
            "POST",
            "/assistant/actions/key-rotate",
            Some(json!({
                "actionRequestId": "rotate-cross-owner",
                "keyId": &other_predecessor_id,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{hidden}");

        for body in [
            json!({ "actionRequestId": "missing-key" }),
            json!({
                "actionRequestId": "unknown-field",
                "keyId": &local_second_id,
                "successorId": Uuid::new_v4().to_string(),
            }),
            json!({
                "actionRequestId": "invalid/control",
                "keyId": &local_second_id,
            }),
        ] {
            let (status, _) = request(
                app(state.clone()),
                &token,
                "POST",
                "/assistant/actions/key-rotate",
                Some(body),
            )
            .await;
            assert!(
                [StatusCode::BAD_REQUEST, StatusCode::UNPROCESSABLE_ENTITY].contains(&status),
                "unexpected adversarial status: {status}"
            );
        }

        assert!(
            db.collection::<ApiKey>(API_KEYS)
                .find_one(doc! { "_id": &local_second_id })
                .await
                .expect("read local second")
                .expect("local second exists")
                .is_active
        );
        assert!(
            db.collection::<ApiKey>(API_KEYS)
                .find_one(doc! { "_id": &other_predecessor_id })
                .await
                .expect("read other predecessor")
                .expect("other predecessor exists")
                .is_active
        );
    }
}
