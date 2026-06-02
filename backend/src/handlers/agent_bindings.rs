use axum::{
    Json,
    extract::{Path, State},
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::mw::auth::AuthUser;
use crate::services::org_service::OwnerAccess;
use crate::services::{agent_binding_service, audit_service, org_service};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

struct BindingOwnerAccess {
    user_id: String,
    access: OwnerAccess,
    /// The API key's platform label (e.g. "claude-code", "codex"). Cached
    /// here so telemetry emits can avoid a second `ApiKey` fetch; the
    /// key was already loaded in `resolve_binding_owner`.
    platform: Option<String>,
}

/// Resolve the effective owner *and* the caller's `OwnerAccess` for a
/// binding operation. Bindings are 1:1 with an API key, so the binding's
/// "owner" is the key's owner. Members and viewers of an org that owns
/// the key cannot manage its bindings; org admins can.
///
/// The membership's `allowed_service_ids` scope is NOT enforced on the
/// `ApiKey` itself (a NyxID API key is an agent identity, not a
/// service). It IS enforced on the individual binding's
/// `user_service_id` -- create/delete handlers must call
/// `OwnerAccess::allows_resource(&user_service_id)` before mutating.
async fn resolve_binding_owner(
    state: &AppState,
    actor: &str,
    key_id: &str,
    for_write: bool,
) -> AppResult<BindingOwnerAccess> {
    let key = state
        .db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": key_id })
        .await?
        .ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;

    let access = org_service::resolve_owner_access(&state.db, actor, &key.user_id).await?;
    if !access.can_read() {
        return Err(AppError::NotFound("API key not found".to_string()));
    }
    if for_write && !access.can_write() {
        return Err(AppError::OrgRoleInsufficient(
            "you do not have permission to modify bindings on this API key".to_string(),
        ));
    }
    Ok(BindingOwnerAccess {
        user_id: key.user_id,
        access,
        platform: key.platform,
    })
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateBindingRequest {
    pub user_service_id: String,
    pub user_api_key_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BindingResponse {
    pub id: String,
    pub api_key_id: String,
    pub user_service_id: String,
    pub user_api_key_id: String,
    pub service_slug: String,
    pub service_label: String,
    pub credential_label: String,
    pub created_at: String,
    pub updated_at: String,
    /// True when the binding references a missing/inactive service or a
    /// missing credential. Create/delete paths cascade-clean bindings
    /// (see `agent_binding_service::cleanup_bindings_for_user_service`
    /// and `cleanup_bindings_for_credential`), so new data should never
    /// be invalid. This flag surfaces pre-existing orphans from earlier
    /// versions so the frontend can let the user clean them up.
    pub is_invalid: bool,
    /// Short machine-readable reason when `is_invalid` is true:
    /// `missing_service`, `inactive_service`, or `missing_credential`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid_reason: Option<String>,
}

async fn enrich_bindings(
    state: &AppState,
    bindings: Vec<crate::models::agent_service_binding::AgentServiceBinding>,
) -> AppResult<Vec<BindingResponse>> {
    let service_ids: Vec<&str> = bindings
        .iter()
        .map(|binding| binding.user_service_id.as_str())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let credential_ids: Vec<&str> = bindings
        .iter()
        .map(|binding| binding.user_api_key_id.as_str())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let services: Vec<UserService> = if service_ids.is_empty() {
        Vec::new()
    } else {
        state
            .db
            .collection::<UserService>(USER_SERVICES)
            .find(doc! { "_id": { "$in": &service_ids } })
            .await?
            .try_collect()
            .await?
    };
    let endpoints: Vec<UserEndpoint> = if services.is_empty() {
        Vec::new()
    } else {
        let endpoint_ids: Vec<&str> = services
            .iter()
            .map(|service| service.endpoint_id.as_str())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        state
            .db
            .collection::<UserEndpoint>(USER_ENDPOINTS)
            .find(doc! { "_id": { "$in": &endpoint_ids } })
            .await?
            .try_collect()
            .await?
    };
    let credentials: Vec<UserApiKey> = if credential_ids.is_empty() {
        Vec::new()
    } else {
        state
            .db
            .collection::<UserApiKey>(USER_API_KEYS)
            .find(doc! { "_id": { "$in": &credential_ids } })
            .await?
            .try_collect()
            .await?
    };

    let endpoint_labels: HashMap<String, String> = endpoints
        .into_iter()
        .map(|endpoint| (endpoint.id, endpoint.label))
        .collect();
    let service_map: HashMap<String, UserService> = services
        .into_iter()
        .map(|service| (service.id.clone(), service))
        .collect();
    let credential_map: HashMap<String, UserApiKey> = credentials
        .into_iter()
        .map(|credential| (credential.id.clone(), credential))
        .collect();

    Ok(bindings
        .into_iter()
        .map(|binding| {
            let service = service_map.get(&binding.user_service_id);
            let credential = credential_map.get(&binding.user_api_key_id);
            let service_slug = service
                .map(|service| service.slug.clone())
                .unwrap_or_else(|| binding.user_service_id.clone());
            let service_label = service
                .and_then(|service| endpoint_labels.get(&service.endpoint_id).cloned())
                .unwrap_or_else(|| service_slug.clone());
            let credential_label = credential
                .map(|credential| credential.label.clone())
                .unwrap_or_else(|| binding.user_api_key_id.clone());

            // Priority: missing service > inactive service > missing
            // credential. Missing service is the most surprising state
            // since the row will render with a UUID where the name
            // should be; flag it first.
            let (is_invalid, invalid_reason) = match (service, credential) {
                (None, _) => (true, Some("missing_service".to_string())),
                (Some(s), _) if !s.is_active => (true, Some("inactive_service".to_string())),
                (_, None) => (true, Some("missing_credential".to_string())),
                _ => (false, None),
            };

            BindingResponse {
                id: binding.id,
                api_key_id: binding.api_key_id,
                user_service_id: binding.user_service_id,
                user_api_key_id: binding.user_api_key_id,
                service_slug,
                service_label,
                credential_label,
                created_at: binding.created_at.to_rfc3339(),
                updated_at: binding.updated_at.to_rfc3339(),
                is_invalid,
                invalid_reason,
            }
        })
        .collect())
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BindingListResponse {
    pub bindings: Vec<BindingResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeleteBindingResponse {
    pub message: String,
}

#[utoipa::path(
    post,
    path = "/api/v1/api-keys/{key_id}/bindings",
    params(
        ("key_id" = String, Path, description = "API key ID")
    ),
    request_body = CreateBindingRequest,
    responses(
        (status = 200, description = "Created binding", body = BindingResponse),
        (status = 400, description = "Validation error", body = crate::errors::ErrorResponse),
        (status = 404, description = "Not found", body = crate::errors::ErrorResponse),
        (status = 409, description = "Binding already exists", body = crate::errors::ErrorResponse)
    ),
    tag = "Agent Bindings"
)]
/// POST /api/v1/api-keys/{key_id}/bindings
pub async fn create_binding(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path(key_id): Path<String>,
    Json(body): Json<CreateBindingRequest>,
) -> AppResult<Json<BindingResponse>> {
    auth_user.ensure_write_scope()?;

    let actor = auth_user.user_id.to_string();
    // Effective owner = the org user_id for org-owned keys, else the actor.
    // `agent_binding_service::create_binding` then validates that the
    // `user_service_id` and `user_api_key_id` also belong to the same
    // owner, which is the intended invariant: an org-owned agent can
    // only bind to org-owned services and credentials, not personal ones.
    let BindingOwnerAccess {
        user_id,
        access,
        platform,
    } = resolve_binding_owner(&state, &actor, &key_id, true).await?;
    // Per-binding scope check: a scoped admin can only bind services in
    // their `allowed_service_ids` set. The body's `user_service_id` is
    // already in `UserService.id` space, so it can be checked directly.
    if !access.allows_resource(&body.user_service_id) {
        return Err(AppError::NotFound("User service not found".to_string()));
    }
    let binding = agent_binding_service::create_binding(
        &state.db,
        &user_id,
        &key_id,
        &body.user_service_id,
        &body.user_api_key_id,
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "agent_binding_created",
        Some(serde_json::json!({
            "binding_id": &binding.id,
            "api_key_id": &key_id,
            "user_service_id": &body.user_service_id,
            "user_api_key_id": &body.user_api_key_id,
        })),
    );

    let mut responses = enrich_bindings(&state, vec![binding]).await?;
    let response = responses.remove(0);

    // Telemetry: agent_binding.created. The enriched response already
    // carries the resolved `service_slug` (falling back to the raw id when
    // the service row is missing), so reuse it instead of issuing another
    // lookup.
    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::AgentBindingCreated {
            platform,
            service_slug: response.service_slug.clone(),
        },
    );

    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/api-keys/{key_id}/bindings",
    params(
        ("key_id" = String, Path, description = "API key ID")
    ),
    responses(
        (status = 200, description = "List of bindings", body = BindingListResponse),
        (status = 404, description = "API key not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Agent Bindings"
)]
/// GET /api/v1/api-keys/{key_id}/bindings
pub async fn list_bindings(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(key_id): Path<String>,
) -> AppResult<Json<BindingListResponse>> {
    let actor = auth_user.user_id.to_string();
    // Read access: any active member/viewer/admin of the owning org can
    // list bindings on an org-owned key. Scope filtering still applies:
    // a scoped admin only sees bindings whose `user_service_id` is in
    // their `allowed_service_ids` set.
    let BindingOwnerAccess {
        user_id,
        access,
        platform: _,
    } = resolve_binding_owner(&state, &actor, &key_id, false).await?;
    let bindings: Vec<_> = agent_binding_service::list_bindings(&state.db, &user_id, &key_id)
        .await?
        .into_iter()
        .filter(|b| access.allows_resource(&b.user_service_id))
        .collect();
    let bindings = enrich_bindings(&state, bindings).await?;
    Ok(Json(BindingListResponse { bindings }))
}

#[utoipa::path(
    delete,
    path = "/api/v1/api-keys/{key_id}/bindings/{binding_id}",
    params(
        ("key_id" = String, Path, description = "API key ID"),
        ("binding_id" = String, Path, description = "Binding ID")
    ),
    responses(
        (status = 200, description = "Binding deleted", body = DeleteBindingResponse),
        (status = 404, description = "Binding not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Agent Bindings"
)]
/// DELETE /api/v1/api-keys/{key_id}/bindings/{binding_id}
pub async fn delete_binding(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path((key_id, binding_id)): Path<(String, String)>,
) -> AppResult<Json<DeleteBindingResponse>> {
    auth_user.ensure_write_scope()?;

    let actor = auth_user.user_id.to_string();
    let BindingOwnerAccess {
        user_id,
        access,
        platform,
    } = resolve_binding_owner(&state, &actor, &key_id, true).await?;
    // Look up the binding to enforce per-binding scope: a scoped admin
    // can only delete bindings whose `user_service_id` is in their
    // `allowed_service_ids` set.
    let binding =
        agent_binding_service::get_binding(&state.db, &user_id, &key_id, &binding_id).await?;
    if !access.allows_resource(&binding.user_service_id) {
        return Err(AppError::NotFound("Binding not found".to_string()));
    }
    // Resolve the UserService slug BEFORE the delete so the telemetry emit
    // below has a stable slug. Once the cascade cleanup runs we'd have to
    // fall back to the raw id; doing the lookup up-front avoids that.
    // Best-effort: a transient read error here must never turn a
    // successful binding load into a failed DELETE. Fall back to the
    // raw id instead.
    let service_slug_for_telemetry: String = state
        .db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! { "_id": &binding.user_service_id })
        .await
        .ok()
        .flatten()
        .map(|svc| svc.slug)
        .unwrap_or_else(|| binding.user_service_id.clone());
    agent_binding_service::delete_binding(&state.db, &user_id, &key_id, &binding_id).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "agent_binding_deleted",
        Some(serde_json::json!({
            "binding_id": &binding_id,
            "api_key_id": &key_id,
        })),
    );

    // Telemetry: agent_binding.deleted. Slug + platform resolved before
    // the cascade so the emit carries meaningful values even if the
    // `UserService` row is gone by the time the event fires.
    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::AgentBindingDeleted {
            platform,
            service_slug: service_slug_for_telemetry,
        },
    );

    Ok(Json(DeleteBindingResponse {
        message: "Binding deleted".to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::agent_service_binding::AgentServiceBinding;
    use crate::models::api_key::ApiKey;
    use crate::models::org_membership::{
        COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
    };
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::models::user_api_key::UserApiKey;
    use crate::test_utils::{
        connect_test_database, test_app_state, test_auth_user, test_membership, test_user,
        test_user_endpoint, test_user_service,
    };
    use axum::extract::State;
    use chrono::Utc;

    fn tele() -> TelemetryContext {
        TelemetryContext::default()
    }

    fn fixture_api_key(id: &str, user_id: &str) -> ApiKey {
        ApiKey {
            id: id.to_string(),
            user_id: user_id.to_string(),
            name: "test-agent".to_string(),
            key_prefix: "nyxid_ag_test".to_string(),
            key_hash: "deadbeef".repeat(8),
            scopes: String::new(),
            last_used_at: None,
            expires_at: None,
            is_active: true,
            created_at: Utc::now(),
            description: None,
            allowed_service_ids: vec![],
            allowed_node_ids: vec![],
            allow_all_services: true,
            allow_all_nodes: true,
            rate_limit_per_second: None,
            rate_limit_burst: None,
            platform: Some("claude-code".to_string()),
            callback_url: None,
        }
    }

    fn fixture_user_api_key(id: &str, user_id: &str) -> UserApiKey {
        UserApiKey {
            id: id.to_string(),
            user_id: user_id.to_string(),
            label: "test-credential".to_string(),
            credential_type: "api_key".to_string(),
            credential_encrypted: Some(vec![1, 2, 3]),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            token_scopes: None,
            expires_at: None,
            provider_config_id: None,
            connection_id: None,
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            status: "active".to_string(),
            last_used_at: None,
            error_message: None,
            source: None,
            source_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn fixture_binding(
        api_key_id: &str,
        user_id: &str,
        user_service_id: &str,
        user_api_key_id: &str,
    ) -> AgentServiceBinding {
        AgentServiceBinding {
            id: uuid::Uuid::new_v4().to_string(),
            api_key_id: api_key_id.to_string(),
            user_service_id: user_service_id.to_string(),
            user_api_key_id: user_api_key_id.to_string(),
            user_id: user_id.to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    struct Fixtures {
        user_id: String,
        api_key_id: String,
        service_id: String,
        credential_id: String,
    }

    async fn seed_fixtures(db: &mongodb::Database) -> Fixtures {
        let user_id = uuid::Uuid::new_v4().to_string();
        let api_key_id = uuid::Uuid::new_v4().to_string();
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let credential_id = uuid::Uuid::new_v4().to_string();

        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        db.collection::<ApiKey>(API_KEYS)
            .insert_one(fixture_api_key(&api_key_id, &user_id))
            .await
            .unwrap();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(test_user_endpoint(
                &endpoint_id,
                &user_id,
                "Test EP",
                "https://api.example.com",
                None,
                None,
            ))
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(test_user_service(
                &service_id,
                &user_id,
                "test-svc",
                &endpoint_id,
                None,
                None,
            ))
            .await
            .unwrap();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(fixture_user_api_key(&credential_id, &user_id))
            .await
            .unwrap();

        Fixtures {
            user_id,
            api_key_id,
            service_id,
            credential_id,
        }
    }

    #[tokio::test]
    async fn create_and_list_binding() {
        let Some(db) = connect_test_database("h_agent_bind_create_list").await else {
            return;
        };
        let f = seed_fixtures(&db).await;
        let state = test_app_state(db);
        let auth = test_auth_user(&f.user_id);

        let Json(created) = create_binding(
            State(state.clone()),
            auth.clone(),
            tele(),
            Path(f.api_key_id.clone()),
            Json(CreateBindingRequest {
                user_service_id: f.service_id.clone(),
                user_api_key_id: f.credential_id.clone(),
            }),
        )
        .await
        .unwrap();

        assert_eq!(created.api_key_id, f.api_key_id);
        assert_eq!(created.user_service_id, f.service_id);
        assert_eq!(created.user_api_key_id, f.credential_id);
        assert!(!created.is_invalid);

        let Json(list) = list_bindings(State(state), auth, Path(f.api_key_id.clone()))
            .await
            .unwrap();

        assert_eq!(list.bindings.len(), 1);
        assert_eq!(list.bindings[0].id, created.id);
    }

    #[tokio::test]
    async fn create_binding_duplicate_returns_conflict() {
        let Some(db) = connect_test_database("h_agent_bind_dup").await else {
            return;
        };
        let f = seed_fixtures(&db).await;
        let state = test_app_state(db);
        let auth = test_auth_user(&f.user_id);

        let _first = create_binding(
            State(state.clone()),
            auth.clone(),
            tele(),
            Path(f.api_key_id.clone()),
            Json(CreateBindingRequest {
                user_service_id: f.service_id.clone(),
                user_api_key_id: f.credential_id.clone(),
            }),
        )
        .await
        .unwrap();

        let err = create_binding(
            State(state),
            auth,
            tele(),
            Path(f.api_key_id.clone()),
            Json(CreateBindingRequest {
                user_service_id: f.service_id.clone(),
                user_api_key_id: f.credential_id.clone(),
            }),
        )
        .await;

        assert!(err.is_err());
    }

    #[tokio::test]
    async fn delete_binding_succeeds() {
        let Some(db) = connect_test_database("h_agent_bind_delete").await else {
            return;
        };
        let f = seed_fixtures(&db).await;
        let state = test_app_state(db);
        let auth = test_auth_user(&f.user_id);

        let Json(created) = create_binding(
            State(state.clone()),
            auth.clone(),
            tele(),
            Path(f.api_key_id.clone()),
            Json(CreateBindingRequest {
                user_service_id: f.service_id.clone(),
                user_api_key_id: f.credential_id.clone(),
            }),
        )
        .await
        .unwrap();

        let Json(resp) = delete_binding(
            State(state.clone()),
            auth.clone(),
            tele(),
            Path((f.api_key_id.clone(), created.id)),
        )
        .await
        .unwrap();

        assert_eq!(resp.message, "Binding deleted");

        let Json(list) = list_bindings(State(state), auth, Path(f.api_key_id.clone()))
            .await
            .unwrap();

        assert!(list.bindings.is_empty());
    }

    #[tokio::test]
    async fn delete_nonexistent_binding_returns_error() {
        let Some(db) = connect_test_database("h_agent_bind_del_404").await else {
            return;
        };
        let f = seed_fixtures(&db).await;
        let state = test_app_state(db);
        let auth = test_auth_user(&f.user_id);

        let err = delete_binding(
            State(state),
            auth,
            tele(),
            Path((f.api_key_id.clone(), uuid::Uuid::new_v4().to_string())),
        )
        .await;

        assert!(err.is_err());
    }

    #[tokio::test]
    async fn list_bindings_api_key_not_found() {
        let Some(db) = connect_test_database("h_agent_bind_list_404").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let err = list_bindings(State(state), auth, Path(uuid::Uuid::new_v4().to_string())).await;

        assert!(err.is_err());
    }

    #[tokio::test]
    async fn create_binding_invalid_service_returns_error() {
        let Some(db) = connect_test_database("h_agent_bind_bad_svc").await else {
            return;
        };
        let f = seed_fixtures(&db).await;
        let state = test_app_state(db);
        let auth = test_auth_user(&f.user_id);

        let err = create_binding(
            State(state),
            auth,
            tele(),
            Path(f.api_key_id.clone()),
            Json(CreateBindingRequest {
                user_service_id: uuid::Uuid::new_v4().to_string(),
                user_api_key_id: f.credential_id.clone(),
            }),
        )
        .await;

        assert!(err.is_err());
    }

    #[tokio::test]
    async fn create_binding_invalid_credential_returns_error() {
        let Some(db) = connect_test_database("h_agent_bind_bad_cred").await else {
            return;
        };
        let f = seed_fixtures(&db).await;
        let state = test_app_state(db);
        let auth = test_auth_user(&f.user_id);

        let err = create_binding(
            State(state),
            auth,
            tele(),
            Path(f.api_key_id.clone()),
            Json(CreateBindingRequest {
                user_service_id: f.service_id.clone(),
                user_api_key_id: uuid::Uuid::new_v4().to_string(),
            }),
        )
        .await;

        assert!(err.is_err());
    }

    #[tokio::test]
    async fn resolve_binding_owner_allows_org_viewer_read_but_denies_write() {
        let Some(db) = connect_test_database("h_agent_bind_owner_viewer").await else {
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let api_key_id = uuid::Uuid::new_v4().to_string();

        db.collection::<User>(USERS)
            .insert_many([
                test_user(&actor_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .unwrap();
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &actor_id, OrgRole::Viewer, None))
            .await
            .unwrap();
        db.collection::<ApiKey>(API_KEYS)
            .insert_one(fixture_api_key(&api_key_id, &org_id))
            .await
            .unwrap();

        let state = test_app_state(db);
        let owner = resolve_binding_owner(&state, &actor_id, &api_key_id, false)
            .await
            .unwrap();
        assert_eq!(owner.user_id, org_id);
        assert!(owner.access.can_read());
        assert!(!owner.access.can_write());
        assert_eq!(owner.platform.as_deref(), Some("claude-code"));

        let err = match resolve_binding_owner(&state, &actor_id, &api_key_id, true).await {
            Ok(_) => panic!("viewer cannot mutate org-owned API key bindings"),
            Err(err) => err,
        };
        assert!(matches!(
            err,
            AppError::OrgRoleInsufficient(message)
                if message == "you do not have permission to modify bindings on this API key"
        ));
    }

    #[tokio::test]
    async fn create_binding_hides_org_service_outside_admin_scope() {
        let Some(db) = connect_test_database("h_agent_bind_scoped_admin").await else {
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let api_key_id = uuid::Uuid::new_v4().to_string();
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let credential_id = uuid::Uuid::new_v4().to_string();

        db.collection::<User>(USERS)
            .insert_many([
                test_user(&actor_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .unwrap();
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(
                &org_id,
                &actor_id,
                OrgRole::Admin,
                Some(vec![uuid::Uuid::new_v4().to_string()]),
            ))
            .await
            .unwrap();
        db.collection::<ApiKey>(API_KEYS)
            .insert_one(fixture_api_key(&api_key_id, &org_id))
            .await
            .unwrap();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(test_user_endpoint(
                &endpoint_id,
                &org_id,
                "Org Endpoint",
                "https://org.example.com",
                None,
                None,
            ))
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(test_user_service(
                &service_id,
                &org_id,
                "org-service",
                &endpoint_id,
                None,
                None,
            ))
            .await
            .unwrap();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(fixture_user_api_key(&credential_id, &org_id))
            .await
            .unwrap();

        let state = test_app_state(db);
        let err = create_binding(
            State(state),
            test_auth_user(&actor_id),
            tele(),
            Path(api_key_id),
            Json(CreateBindingRequest {
                user_service_id: service_id,
                user_api_key_id: credential_id,
            }),
        )
        .await
        .expect_err("out-of-scope org service should be hidden");

        assert!(matches!(
            err,
            AppError::NotFound(message) if message == "User service not found"
        ));
    }

    #[tokio::test]
    async fn enrich_bindings_marks_missing_and_inactive_references() {
        let Some(db) = connect_test_database("h_agent_bind_enrich_invalid").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let api_key_id = uuid::Uuid::new_v4().to_string();
        let active_endpoint_id = uuid::Uuid::new_v4().to_string();
        let inactive_endpoint_id = uuid::Uuid::new_v4().to_string();
        let active_service_id = uuid::Uuid::new_v4().to_string();
        let inactive_service_id = uuid::Uuid::new_v4().to_string();
        let credential_id = uuid::Uuid::new_v4().to_string();
        let missing_service_id = uuid::Uuid::new_v4().to_string();
        let missing_credential_id = uuid::Uuid::new_v4().to_string();

        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_many([
                test_user_endpoint(
                    &active_endpoint_id,
                    &user_id,
                    "Active Endpoint",
                    "https://active.example.com",
                    None,
                    None,
                ),
                test_user_endpoint(
                    &inactive_endpoint_id,
                    &user_id,
                    "Inactive Endpoint",
                    "https://inactive.example.com",
                    None,
                    None,
                ),
            ])
            .await
            .unwrap();
        let active_service = test_user_service(
            &active_service_id,
            &user_id,
            "active-service",
            &active_endpoint_id,
            None,
            None,
        );
        let mut inactive_service = test_user_service(
            &inactive_service_id,
            &user_id,
            "inactive-service",
            &inactive_endpoint_id,
            None,
            None,
        );
        inactive_service.is_active = false;
        db.collection::<UserService>(USER_SERVICES)
            .insert_many([active_service, inactive_service])
            .await
            .unwrap();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(fixture_user_api_key(&credential_id, &user_id))
            .await
            .unwrap();

        let state = test_app_state(db);
        let responses = enrich_bindings(
            &state,
            vec![
                fixture_binding(&api_key_id, &user_id, &missing_service_id, &credential_id),
                fixture_binding(&api_key_id, &user_id, &inactive_service_id, &credential_id),
                fixture_binding(
                    &api_key_id,
                    &user_id,
                    &active_service_id,
                    &missing_credential_id,
                ),
            ],
        )
        .await
        .unwrap();

        assert_eq!(responses.len(), 3);
        assert_eq!(responses[0].service_slug, missing_service_id);
        assert_eq!(responses[0].service_label, responses[0].service_slug);
        assert_eq!(responses[0].credential_label, "test-credential");
        assert!(responses[0].is_invalid);
        assert_eq!(
            responses[0].invalid_reason.as_deref(),
            Some("missing_service")
        );

        assert_eq!(responses[1].service_slug, "inactive-service");
        assert_eq!(responses[1].service_label, "Inactive Endpoint");
        assert_eq!(responses[1].credential_label, "test-credential");
        assert!(responses[1].is_invalid);
        assert_eq!(
            responses[1].invalid_reason.as_deref(),
            Some("inactive_service")
        );

        assert_eq!(responses[2].service_slug, "active-service");
        assert_eq!(responses[2].service_label, "Active Endpoint");
        assert_eq!(responses[2].credential_label, missing_credential_id);
        assert!(responses[2].is_invalid);
        assert_eq!(
            responses[2].invalid_reason.as_deref(),
            Some("missing_credential")
        );
    }
}
