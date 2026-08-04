use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use mongodb::bson::{self, doc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
use crate::models::user_service::COLLECTION_NAME as USER_SERVICES;
use crate::mw::auth::AuthUser;
use crate::services::{
    api_docs_service, openapi_parser, org_service, user_endpoint_service, user_service_service,
};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

/// Resolve which user_id owns this endpoint and whether the actor may
/// modify it. Returns the effective owner_id (may be an org user id) for
/// downstream service calls. Errors out as Forbidden / NotFound otherwise.
///
/// `OrgMembership.allowed_service_ids` is keyed by `UserService.id`, not
/// by endpoint id. We translate by looking up every UserService that
/// references this endpoint and gating on `allows_any_resource`. An
/// orphan endpoint (referenced by zero services) is treated as a
/// scope-less resource: only Direct owners or unscoped admins can touch
/// it, since a scoped admin has no concrete claim to it.
async fn resolve_endpoint_write_owner(
    state: &AppState,
    actor: &str,
    endpoint_id: &str,
) -> AppResult<String> {
    let endpoint = state
        .db
        .collection::<UserEndpoint>(USER_ENDPOINTS)
        .find_one(doc! { "_id": endpoint_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))?;

    let access = org_service::resolve_owner_access(&state.db, actor, &endpoint.user_id).await?;
    if !access.can_read() {
        return Err(AppError::NotFound("Endpoint not found".to_string()));
    }
    let backing_service_ids = user_service_service::user_service_ids_for_endpoint(
        &state.db,
        &endpoint.user_id,
        &endpoint.id,
    )
    .await?;
    if !access.allows_any_resource(&backing_service_ids) {
        return Err(AppError::NotFound("Endpoint not found".to_string()));
    }
    if !access.can_write() {
        return Err(AppError::OrgRoleInsufficient(
            "you do not have permission to modify this endpoint".to_string(),
        ));
    }
    Ok(endpoint.user_id)
}

async fn endpoint_is_only_node_routed(
    state: &AppState,
    owner_id: &str,
    endpoint_id: &str,
) -> AppResult<bool> {
    let services = state
        .db
        .collection::<mongodb::bson::Document>(USER_SERVICES);
    let total_count = services
        .count_documents(doc! { "user_id": owner_id, "endpoint_id": endpoint_id })
        .await?;
    if total_count == 0 {
        return Ok(false);
    }

    let direct_count = services
        .count_documents(doc! {
            "user_id": owner_id,
            "endpoint_id": endpoint_id,
            "$or": [
                { "node_id": { "$exists": false } },
                { "node_id": bson::Bson::Null },
                { "node_id": "" },
            ],
        })
        .await?;

    Ok(direct_count == 0)
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateEndpointRequest {
    pub url: Option<String>,
    pub label: Option<String>,
    /// Optional OpenAPI spec URL for endpoint discovery. Sending `""`
    /// clears the field; omitting leaves the current value untouched.
    pub openapi_spec_url: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EndpointResponse {
    pub id: String,
    pub label: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_service_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openapi_spec_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EndpointListResponse {
    pub endpoints: Vec<EndpointResponse>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct EndpointListQuery {
    /// When set, list endpoints owned by the given org instead of the
    /// caller's personal endpoints. Caller must be an admin of the org
    /// (admin role required so orphan endpoints blocking org deletion
    /// can be cleaned up, matching the `?org_id=` contract on other
    /// org-scoped list endpoints).
    pub org_id: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/endpoints",
    params(EndpointListQuery),
    responses(
        (status = 200, description = "List of user endpoints", body = EndpointListResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 403, description = "Not an admin of the target org", body = crate::errors::ErrorResponse),
        (status = 404, description = "Org not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Endpoints"
)]
/// GET /api/v1/endpoints
///
/// Defaults to listing the caller's personal endpoints. Pass
/// `?org_id=<id>` to list endpoints owned by an org (the caller must be
/// an admin of that org). This is how admins discover orphan endpoints
/// that block org deletion (issue #365).
pub async fn list_endpoints(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<EndpointListQuery>,
) -> AppResult<Json<EndpointListResponse>> {
    let actor = auth_user.user_id.to_string();
    let user_id_str = if let Some(target_org_id) = query.org_id.as_deref() {
        let access = org_service::resolve_owner_access(&state.db, &actor, target_org_id).await?;
        if !access.can_write() {
            return Err(AppError::OrgRoleInsufficient(
                "admin access to the target org is required to list its endpoints".to_string(),
            ));
        }
        target_org_id.to_string()
    } else {
        actor
    };
    let endpoints = user_endpoint_service::list_endpoints(&state.db, &user_id_str).await?;
    let items = endpoints.into_iter().map(endpoint_response).collect();
    Ok(Json(EndpointListResponse { endpoints: items }))
}

#[utoipa::path(
    put,
    path = "/api/v1/endpoints/{endpoint_id}",
    params(
        ("endpoint_id" = String, Path, description = "User endpoint ID")
    ),
    request_body = UpdateEndpointRequest,
    responses(
        (status = 200, description = "Updated endpoint", body = EndpointResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Endpoint not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Endpoints"
)]
/// PUT /api/v1/endpoints/{endpoint_id}
pub async fn update_endpoint(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path(endpoint_id): Path<String>,
    Json(body): Json<UpdateEndpointRequest>,
) -> AppResult<Json<EndpointResponse>> {
    let actor = auth_user.user_id.to_string();
    let owner_id = resolve_endpoint_write_owner(&state, &actor, &endpoint_id).await?;

    if let Some(url) = body.url.as_deref()
        && !endpoint_is_only_node_routed(&state, &owner_id, &endpoint_id).await?
    {
        crate::services::url_validation::validate_user_endpoint_url(
            url,
            state.config.is_production(),
            "endpoint_url",
        )
        .await?;
    }

    let spec_update = match body.openapi_spec_url.as_deref() {
        None => user_endpoint_service::OpenApiSpecUrlUpdate::Leave,
        Some(s) if s.trim().is_empty() => user_endpoint_service::OpenApiSpecUrlUpdate::Clear,
        Some(s) => user_endpoint_service::OpenApiSpecUrlUpdate::Set(s),
    };
    user_endpoint_service::update_endpoint(
        &state.db,
        &owner_id,
        &endpoint_id,
        body.url.as_deref(),
        body.label.as_deref(),
        spec_update,
    )
    .await?;

    let ep = user_endpoint_service::get_endpoint(&state.db, &owner_id, &endpoint_id).await?;

    // Telemetry: endpoint.updated. `endpoint_type` is "catalog" when this
    // endpoint was auto-provisioned from a catalog template (i.e. has a
    // `catalog_service_id`), else "custom" for user-defined URLs.
    let endpoint_type = if ep.catalog_service_id.is_some() {
        "catalog"
    } else {
        "custom"
    };
    emit_event(
        state.telemetry.as_deref(),
        &actor,
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::EndpointUpdated {
            endpoint_type: endpoint_type.to_string(),
        },
    );

    Ok(Json(endpoint_response(ep)))
}

#[utoipa::path(
    delete,
    path = "/api/v1/endpoints/{endpoint_id}",
    params(
        ("endpoint_id" = String, Path, description = "User endpoint ID")
    ),
    responses(
        (status = 204, description = "Endpoint deleted"),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Endpoint not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Endpoints"
)]
/// DELETE /api/v1/endpoints/{endpoint_id}
pub async fn delete_endpoint(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path(endpoint_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let actor = auth_user.user_id.to_string();
    let owner_id = resolve_endpoint_write_owner(&state, &actor, &endpoint_id).await?;

    // Capture endpoint_type pre-delete so telemetry can classify custom vs
    // catalog-derived endpoints even though the record will be gone after
    // the delete call.
    let endpoint_type = user_endpoint_service::get_endpoint(&state.db, &owner_id, &endpoint_id)
        .await
        .ok()
        .map(|ep| {
            if ep.catalog_service_id.is_some() {
                "catalog"
            } else {
                "custom"
            }
        })
        .unwrap_or("custom");

    user_endpoint_service::delete_endpoint(&state.db, &owner_id, &endpoint_id).await?;

    // Telemetry: endpoint.deleted.
    emit_event(
        state.telemetry.as_deref(),
        &actor,
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::EndpointDeleted {
            endpoint_type: endpoint_type.to_string(),
        },
    );

    Ok(StatusCode::NO_CONTENT)
}

fn endpoint_response(ep: UserEndpoint) -> EndpointResponse {
    EndpointResponse {
        id: ep.id,
        label: ep.label,
        url: ep.url,
        catalog_service_id: ep.catalog_service_id,
        openapi_spec_url: ep.openapi_spec_url,
        created_at: ep.created_at.to_rfc3339(),
        updated_at: ep.updated_at.to_rfc3339(),
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserEndpointOperationResponse {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub method: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_body_schema: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_content_type: Option<String>,
    pub request_body_required: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserEndpointOperationsResponse {
    pub endpoint_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openapi_spec_url: Option<String>,
    pub operations: Vec<UserEndpointOperationResponse>,
}

fn parsed_endpoint_to_response(p: openapi_parser::ParsedEndpoint) -> UserEndpointOperationResponse {
    UserEndpointOperationResponse {
        name: p.name,
        description: p.description,
        method: p.method,
        path: p.path,
        parameters: p.parameters,
        request_body_schema: p.request_body_schema,
        request_content_type: p.request_content_type,
        request_body_required: p.request_body_required,
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/endpoints/{endpoint_id}/openapi-endpoints",
    params(
        ("endpoint_id" = String, Path, description = "User endpoint ID")
    ),
    responses(
        (status = 200, description = "Parsed operations from the user endpoint's OpenAPI spec", body = UserEndpointOperationsResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Endpoint not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Endpoints"
)]
/// GET /api/v1/endpoints/{endpoint_id}/openapi-endpoints
pub async fn list_openapi_endpoints(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(endpoint_id): Path<String>,
) -> AppResult<Json<UserEndpointOperationsResponse>> {
    let actor = auth_user.user_id.to_string();

    // Ownership check: mirror the read-access path used by list/update.
    let endpoint = state
        .db
        .collection::<UserEndpoint>(USER_ENDPOINTS)
        .find_one(doc! { "_id": &endpoint_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))?;

    let access = org_service::resolve_owner_access(&state.db, &actor, &endpoint.user_id).await?;
    if !access.can_read() {
        return Err(AppError::NotFound("Endpoint not found".to_string()));
    }
    let backing_service_ids = user_service_service::user_service_ids_for_endpoint(
        &state.db,
        &endpoint.user_id,
        &endpoint.id,
    )
    .await?;
    if !access.allows_any_resource(&backing_service_ids) {
        return Err(AppError::NotFound("Endpoint not found".to_string()));
    }

    let Some(ref spec_url) = endpoint.openapi_spec_url else {
        return Ok(Json(UserEndpointOperationsResponse {
            endpoint_id: endpoint.id,
            openapi_spec_url: None,
            operations: vec![],
        }));
    };

    // Scope the cache by the *owning* user_id so private specs don't leak
    // between users. Reuses the hardened fetch path: DNS pinning, 5 MB cap,
    // no redirects, 60 s TTL.
    let spec = api_docs_service::fetch_spec_json_scoped(spec_url, &endpoint.user_id).await?;
    let parsed = openapi_parser::parse_openapi_spec_value(&spec)?;
    let operations = parsed
        .into_iter()
        .map(parsed_endpoint_to_response)
        .collect();

    Ok(Json(UserEndpointOperationsResponse {
        endpoint_id: endpoint.id,
        openapi_spec_url: Some(spec_url.clone()),
        operations,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::models::user_endpoint::{COLLECTION_NAME as EP_COLLECTION, UserEndpoint};
    use crate::test_utils::{
        connect_test_database, test_app_state, test_auth_user, test_user, test_user_endpoint,
    };
    use axum::extract::State;

    fn tele() -> TelemetryContext {
        TelemetryContext::default()
    }

    #[tokio::test]
    async fn list_endpoints_returns_user_endpoints() {
        let Some(db) = connect_test_database("h_user_ep_list").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let ep_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        db.collection::<UserEndpoint>(EP_COLLECTION)
            .insert_one(test_user_endpoint(
                &ep_id,
                &user_id,
                "My Endpoint",
                "https://api.example.com",
                None,
                None,
            ))
            .await
            .unwrap();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let Json(resp) = list_endpoints(
            State(state),
            auth,
            Query(EndpointListQuery { org_id: None }),
        )
        .await
        .unwrap();

        assert_eq!(resp.endpoints.len(), 1);
        assert_eq!(resp.endpoints[0].id, ep_id);
        assert_eq!(resp.endpoints[0].label, "My Endpoint");
    }

    #[tokio::test]
    async fn list_endpoints_empty_for_new_user() {
        let Some(db) = connect_test_database("h_user_ep_list_empty").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let Json(resp) = list_endpoints(
            State(state),
            auth,
            Query(EndpointListQuery { org_id: None }),
        )
        .await
        .unwrap();

        assert!(resp.endpoints.is_empty());
    }

    #[tokio::test]
    async fn update_endpoint_label() {
        let Some(db) = connect_test_database("h_user_ep_update").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let ep_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        db.collection::<UserEndpoint>(EP_COLLECTION)
            .insert_one(test_user_endpoint(
                &ep_id,
                &user_id,
                "Old Label",
                "https://api.example.com",
                None,
                None,
            ))
            .await
            .unwrap();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let Json(updated) = update_endpoint(
            State(state),
            auth,
            tele(),
            Path(ep_id.clone()),
            Json(UpdateEndpointRequest {
                url: None,
                label: Some("New Label".to_string()),
                openapi_spec_url: None,
            }),
        )
        .await
        .unwrap();

        assert_eq!(updated.id, ep_id);
        assert_eq!(updated.label, "New Label");
    }

    #[tokio::test]
    async fn update_nonexistent_endpoint_returns_not_found() {
        let Some(db) = connect_test_database("h_user_ep_update_404").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let err = update_endpoint(
            State(state),
            auth,
            tele(),
            Path(uuid::Uuid::new_v4().to_string()),
            Json(UpdateEndpointRequest {
                url: None,
                label: Some("Nope".to_string()),
                openapi_spec_url: None,
            }),
        )
        .await;

        assert!(err.is_err());
    }

    #[tokio::test]
    async fn delete_endpoint_succeeds() {
        let Some(db) = connect_test_database("h_user_ep_delete").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let ep_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        db.collection::<UserEndpoint>(EP_COLLECTION)
            .insert_one(test_user_endpoint(
                &ep_id,
                &user_id,
                "To Delete",
                "https://api.example.com",
                None,
                None,
            ))
            .await
            .unwrap();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let resp = delete_endpoint(
            State(state.clone()),
            auth.clone(),
            tele(),
            Path(ep_id.clone()),
        )
        .await;

        assert!(resp.is_ok());

        let Json(list) = list_endpoints(
            State(state),
            auth,
            Query(EndpointListQuery { org_id: None }),
        )
        .await
        .unwrap();

        assert!(list.endpoints.is_empty());
    }

    #[test]
    fn endpoint_response_maps_all_fields() {
        let ep = UserEndpoint {
            id: "ep-1".into(),
            user_id: "u-1".into(),
            label: "My EP".into(),
            url: "https://api.example.com/v1".into(),
            catalog_service_id: Some("cat-1".into()),
            openapi_spec_url: Some("https://api.example.com/openapi.json".into()),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let resp = endpoint_response(ep);
        assert_eq!(resp.id, "ep-1");
        assert_eq!(resp.label, "My EP");
        assert_eq!(resp.url, "https://api.example.com/v1");
        assert_eq!(resp.catalog_service_id.as_deref(), Some("cat-1"));
        assert_eq!(
            resp.openapi_spec_url.as_deref(),
            Some("https://api.example.com/openapi.json")
        );
    }

    #[test]
    fn endpoint_response_handles_none_optionals() {
        let ep = UserEndpoint {
            id: "ep-2".into(),
            user_id: "u-2".into(),
            label: "Minimal".into(),
            url: "https://example.com".into(),
            catalog_service_id: None,
            openapi_spec_url: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let resp = endpoint_response(ep);
        assert!(resp.catalog_service_id.is_none());
        assert!(resp.openapi_spec_url.is_none());
    }

    #[test]
    fn parsed_endpoint_to_response_maps_all_fields() {
        let parsed = openapi_parser::ParsedEndpoint {
            name: "list_users".into(),
            description: Some("List all users".into()),
            method: "GET".into(),
            path: "/users".into(),
            parameters: Some(serde_json::json!([{"name": "limit", "in": "query"}])),
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
        };
        let resp = parsed_endpoint_to_response(parsed);
        assert_eq!(resp.name, "list_users");
        assert_eq!(resp.description.as_deref(), Some("List all users"));
        assert_eq!(resp.method, "GET");
        assert_eq!(resp.path, "/users");
        assert!(resp.parameters.is_some());
        assert!(!resp.request_body_required);
    }

    #[test]
    fn parsed_endpoint_to_response_with_body() {
        let parsed = openapi_parser::ParsedEndpoint {
            name: "create_user".into(),
            description: None,
            method: "POST".into(),
            path: "/users".into(),
            parameters: None,
            request_body_schema: Some(serde_json::json!({"type": "object"})),
            request_content_type: Some("application/json".into()),
            request_body_required: true,
        };
        let resp = parsed_endpoint_to_response(parsed);
        assert_eq!(resp.name, "create_user");
        assert!(resp.description.is_none());
        assert!(resp.request_body_schema.is_some());
        assert_eq!(
            resp.request_content_type.as_deref(),
            Some("application/json")
        );
        assert!(resp.request_body_required);
    }

    #[test]
    fn endpoint_response_serializes_without_none_fields() {
        let ep = UserEndpoint {
            id: "ep-3".into(),
            user_id: "u-3".into(),
            label: "Test".into(),
            url: "https://example.com".into(),
            catalog_service_id: None,
            openapi_spec_url: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let resp = endpoint_response(ep);
        let json = serde_json::to_value(&resp).unwrap();
        assert!(!json.as_object().unwrap().contains_key("catalog_service_id"));
        assert!(!json.as_object().unwrap().contains_key("openapi_spec_url"));
    }

    #[tokio::test]
    async fn list_openapi_endpoints_no_spec_returns_empty() {
        let Some(db) = connect_test_database("h_user_ep_openapi_empty").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let ep_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        db.collection::<UserEndpoint>(EP_COLLECTION)
            .insert_one(test_user_endpoint(
                &ep_id,
                &user_id,
                "No Spec EP",
                "https://api.example.com",
                None,
                None,
            ))
            .await
            .unwrap();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let Json(resp) = list_openapi_endpoints(State(state), auth, Path(ep_id.clone()))
            .await
            .unwrap();

        assert_eq!(resp.endpoint_id, ep_id);
        assert!(resp.openapi_spec_url.is_none());
        assert!(resp.operations.is_empty());
    }
}
