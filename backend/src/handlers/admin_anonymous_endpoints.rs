use axum::{
    Json,
    extract::{Path, State},
};
use chrono::Utc;
use mongodb::bson::{self, doc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    AnonymousEndpointRule, COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::mw::auth::AuthUser;
use crate::services::{anonymous_endpoint_service, audit_service};

use super::services_helpers::{fetch_service, require_admin_or_creator};

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAnonymousEndpointRequest {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub method: String,
    pub path_pattern: String,
    #[serde(default = "default_daily_quota")]
    pub daily_quota: u32,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateAnonymousEndpointRequest {
    pub enabled: Option<bool>,
    pub method: Option<String>,
    pub path_pattern: Option<String>,
    pub daily_quota: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AnonymousEndpointResponse {
    pub id: String,
    pub enabled: bool,
    pub method: String,
    pub path_pattern: String,
    pub daily_quota: u32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AnonymousEndpointListResponse {
    pub endpoints: Vec<AnonymousEndpointResponse>,
}

fn default_enabled() -> bool {
    true
}

fn default_daily_quota() -> u32 {
    1_000
}

fn response(rule: AnonymousEndpointRule) -> AnonymousEndpointResponse {
    AnonymousEndpointResponse {
        id: rule.id,
        enabled: rule.enabled,
        method: rule.method,
        path_pattern: rule.path_pattern,
        daily_quota: rule.daily_quota,
    }
}

pub async fn list_anonymous_endpoints(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<AnonymousEndpointListResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;
    Ok(Json(AnonymousEndpointListResponse {
        endpoints: service
            .anonymous_endpoints
            .into_iter()
            .map(response)
            .collect(),
    }))
}

pub async fn create_anonymous_endpoint(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    Json(body): Json<CreateAnonymousEndpointRequest>,
) -> AppResult<Json<AnonymousEndpointResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;

    let rule =
        anonymous_endpoint_service::build_rule(anonymous_endpoint_service::AnonymousRuleInput {
            enabled: body.enabled,
            method: body.method,
            path_pattern: body.path_pattern,
            daily_quota: body.daily_quota,
        })?;
    let mut rules = service.anonymous_endpoints.clone();
    rules.push(rule.clone());
    anonymous_endpoint_service::validate_rules_for_service(&service, &rules)?;
    persist_rules(&state, &service_id, &rules).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "anonymous_endpoint_created",
        Some(serde_json::json!({
            "service_id": service_id,
            "rule_id": rule.id,
            "enabled": rule.enabled,
            "method": rule.method,
            "path_pattern": rule.path_pattern,
            "daily_quota": rule.daily_quota,
        })),
    );

    Ok(Json(response(rule)))
}

pub async fn update_anonymous_endpoint(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((service_id, rule_id)): Path<(String, String)>,
    Json(body): Json<UpdateAnonymousEndpointRequest>,
) -> AppResult<Json<AnonymousEndpointResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;

    let mut rules = service.anonymous_endpoints.clone();
    let index = rules
        .iter()
        .position(|rule| rule.id == rule_id)
        .ok_or_else(|| AppError::NotFound("Anonymous endpoint not found".to_string()))?;
    let updated = anonymous_endpoint_service::apply_rule_update(
        &rules[index],
        anonymous_endpoint_service::AnonymousRuleUpdate {
            enabled: body.enabled,
            method: body.method,
            path_pattern: body.path_pattern,
            daily_quota: body.daily_quota,
        },
    )?;
    rules[index] = updated.clone();
    anonymous_endpoint_service::validate_rules_for_service(&service, &rules)?;
    persist_rules(&state, &service_id, &rules).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "anonymous_endpoint_updated",
        Some(serde_json::json!({
            "service_id": service_id,
            "rule_id": updated.id,
            "enabled": updated.enabled,
            "method": updated.method,
            "path_pattern": updated.path_pattern,
            "daily_quota": updated.daily_quota,
        })),
    );

    Ok(Json(response(updated)))
}

pub async fn delete_anonymous_endpoint(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((service_id, rule_id)): Path<(String, String)>,
) -> AppResult<Json<AnonymousEndpointListResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;

    let mut rules = service.anonymous_endpoints.clone();
    let before = rules.len();
    rules.retain(|rule| rule.id != rule_id);
    if rules.len() == before {
        return Err(AppError::NotFound(
            "Anonymous endpoint not found".to_string(),
        ));
    }
    anonymous_endpoint_service::validate_rules_for_service(&service, &rules)?;
    persist_rules(&state, &service_id, &rules).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "anonymous_endpoint_deleted",
        Some(serde_json::json!({
            "service_id": service_id,
            "rule_id": rule_id,
        })),
    );

    Ok(Json(AnonymousEndpointListResponse {
        endpoints: rules.into_iter().map(response).collect(),
    }))
}

async fn persist_rules(
    state: &AppState,
    service_id: &str,
    rules: &[AnonymousEndpointRule],
) -> AppResult<()> {
    let rules_bson = bson::to_bson(rules)
        .map_err(|e| AppError::Internal(format!("BSON serialization error: {e}")))?;
    state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .update_one(
            doc! { "_id": service_id },
            doc! { "$set": {
                "anonymous_endpoints": rules_bson,
                "updated_at": bson::DateTime::from_chrono(Utc::now()),
            }},
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::services::role_service;
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};
    use uuid::Uuid;

    /// Build a catalog `DownstreamService`. `identity_propagating` toggles
    /// whether the service violates the fail-closed runtime-safety contract
    /// (identity_propagation_mode != "none" or token forwarding/delegation on).
    fn catalog_service(created_by: &str, identity_propagating: bool) -> DownstreamService {
        DownstreamService {
            id: Uuid::new_v4().to_string(),
            name: "Catalog".to_string(),
            slug: format!("svc-{}", Uuid::new_v4().simple()),
            description: None,
            base_url: "https://example.test".to_string(),
            service_type: "http".to_string(),
            visibility: "public".to_string(),
            auth_method: if identity_propagating {
                "bearer"
            } else {
                "none"
            }
            .to_string(),
            auth_key_name: if identity_propagating {
                "Authorization".to_string()
            } else {
                String::new()
            },
            credential_encrypted: vec![],
            auth_type: None,
            openapi_spec_url: None,
            asyncapi_spec_url: None,
            streaming_supported: false,
            ssh_config: None,
            oauth_client_id: None,
            service_category: "internal".to_string(),
            requires_user_credential: false,
            is_active: true,
            created_by: created_by.to_string(),
            identity_propagation_mode: if identity_propagating {
                "headers"
            } else {
                "none"
            }
            .to_string(),
            identity_include_user_id: identity_propagating,
            identity_include_email: false,
            identity_include_name: false,
            identity_jwt_audience: None,
            forward_access_token: identity_propagating,
            inject_delegation_token: false,
            delegation_token_scope: "llm:proxy".to_string(),
            provider_config_id: None,
            homepage_url: None,
            repository_url: None,
            issues_url: None,
            capabilities: None,
            auth_notes: None,
            known_limitations: None,
            required_permissions: None,
            examples_url: None,
            recommended_skills: None,
            custom_user_agent: None,
            default_request_headers: None,
            ws_frame_injections: Vec::new(),
            developer_app_ids: None,
            token_exchange_config: None,
            anonymous_endpoints: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    async fn insert_admin(db: &mongodb::Database) -> String {
        role_service::seed_system_roles(db)
            .await
            .expect("seed platform roles");
        let platform_role_ids = role_service::get_platform_role_ids(db)
            .await
            .expect("platform role ids");
        let id = Uuid::new_v4().to_string();
        let mut user = test_user(&id, UserType::Person);
        user.role_ids.push(platform_role_ids.admin);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .expect("insert admin user");
        id
    }

    async fn insert_user(db: &mongodb::Database) -> String {
        // Platform roles must be seeded so `require_admin_or_creator` can
        // resolve a non-admin caller's platform role instead of erroring on a
        // missing 'admin' system role.
        role_service::seed_system_roles(db)
            .await
            .expect("seed platform roles");
        let id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&id, UserType::Person))
            .await
            .expect("insert user");
        id
    }

    async fn insert_service(db: &mongodb::Database, service: &DownstreamService) {
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(service)
            .await
            .expect("insert service");
    }

    async fn load_service(db: &mongodb::Database, service_id: &str) -> DownstreamService {
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find_one(doc! { "_id": service_id })
            .await
            .expect("query service")
            .expect("service exists")
    }

    /// Creating an *enabled* anonymous rule on an identity-propagating
    /// service is fail-closed: it must be rejected with
    /// `AnonymousIncompatibleService` (HTTP 400 / code 11100) and nothing
    /// must be persisted.
    #[tokio::test]
    async fn create_enabled_rule_on_identity_service_is_rejected() {
        let Some(db) = connect_test_database("anon_create_enabled_identity").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let service = catalog_service(&admin_id, /* identity_propagating */ true);
        insert_service(&db, &service).await;
        let service_id = service.id.clone();
        let state = test_app_state(db.clone());

        let err = create_anonymous_endpoint(
            State(state),
            test_auth_user(&admin_id),
            Path(service_id.clone()),
            Json(CreateAnonymousEndpointRequest {
                enabled: true,
                method: "GET".to_string(),
                path_pattern: "/public/**".to_string(),
                daily_quota: 100,
            }),
        )
        .await
        .expect_err("enabled rule on identity service must be rejected");

        assert!(
            matches!(err, AppError::AnonymousIncompatibleService(_)),
            "expected AnonymousIncompatibleService, got {err:?}"
        );
        assert_eq!(err.error_code(), 11100);

        // Fail-closed: rejected write must not persist a rule.
        let persisted = load_service(&db, &service_id).await;
        assert!(persisted.anonymous_endpoints.is_empty());
    }

    /// A *disabled* draft rule may be stored on any service, including an
    /// identity-propagating one (it is inert until enabled).
    #[tokio::test]
    async fn create_disabled_draft_rule_allowed_on_identity_service() {
        let Some(db) = connect_test_database("anon_create_disabled_draft").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let service = catalog_service(&admin_id, /* identity_propagating */ true);
        insert_service(&db, &service).await;
        let service_id = service.id.clone();
        let state = test_app_state(db.clone());

        let response = create_anonymous_endpoint(
            State(state),
            test_auth_user(&admin_id),
            Path(service_id.clone()),
            Json(CreateAnonymousEndpointRequest {
                enabled: false,
                method: "GET".to_string(),
                path_pattern: "/public/**".to_string(),
                daily_quota: 100,
            }),
        )
        .await
        .expect("disabled draft rule must be allowed");

        assert!(!response.0.enabled);

        // Persistence: the disabled rule is durably stored.
        let persisted = load_service(&db, &service_id).await;
        assert_eq!(persisted.anonymous_endpoints.len(), 1);
        assert!(!persisted.anonymous_endpoints[0].enabled);
        assert_eq!(persisted.anonymous_endpoints[0].id, response.0.id);
    }

    /// Creating an enabled rule on a safe service persists it and round-trips.
    #[tokio::test]
    async fn create_enabled_rule_on_safe_service_persists() {
        let Some(db) = connect_test_database("anon_create_enabled_safe").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let service = catalog_service(&admin_id, /* identity_propagating */ false);
        insert_service(&db, &service).await;
        let service_id = service.id.clone();
        let state = test_app_state(db.clone());

        let response = create_anonymous_endpoint(
            State(state),
            test_auth_user(&admin_id),
            Path(service_id.clone()),
            Json(CreateAnonymousEndpointRequest {
                enabled: true,
                method: "POST".to_string(),
                path_pattern: "/public/**".to_string(),
                daily_quota: 50,
            }),
        )
        .await
        .expect("enabled rule on safe service must be allowed");

        assert!(response.0.enabled);
        assert_eq!(response.0.method, "POST");
        assert_eq!(response.0.daily_quota, 50);

        let persisted = load_service(&db, &service_id).await;
        assert_eq!(persisted.anonymous_endpoints.len(), 1);
        assert!(persisted.anonymous_endpoints[0].enabled);
        assert_eq!(persisted.anonymous_endpoints[0].path_pattern, "/public/**");
    }

    /// Enabling a previously-disabled draft rule on an identity-propagating
    /// service via update is fail-closed: rejected and not persisted.
    #[tokio::test]
    async fn enabling_draft_rule_on_identity_service_is_rejected() {
        let Some(db) = connect_test_database("anon_enable_draft_identity").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let mut service = catalog_service(&admin_id, /* identity_propagating */ true);
        let rule = AnonymousEndpointRule {
            id: Uuid::new_v4().to_string(),
            enabled: false,
            method: "GET".to_string(),
            path_pattern: "/public/**".to_string(),
            daily_quota: 100,
        };
        service.anonymous_endpoints.push(rule.clone());
        insert_service(&db, &service).await;
        let service_id = service.id.clone();
        let state = test_app_state(db.clone());

        let err = update_anonymous_endpoint(
            State(state),
            test_auth_user(&admin_id),
            Path((service_id.clone(), rule.id.clone())),
            Json(UpdateAnonymousEndpointRequest {
                enabled: Some(true),
                method: None,
                path_pattern: None,
                daily_quota: None,
            }),
        )
        .await
        .expect_err("enabling a draft on an identity service must be rejected");

        assert!(matches!(err, AppError::AnonymousIncompatibleService(_)));
        assert_eq!(err.error_code(), 11100);

        // Fail-closed: the rule remains disabled in storage.
        let persisted = load_service(&db, &service_id).await;
        assert_eq!(persisted.anonymous_endpoints.len(), 1);
        assert!(!persisted.anonymous_endpoints[0].enabled);
    }

    /// A non-admin, non-owner caller is denied (ACL enforcement) on every
    /// CRUD verb before any rule mutation happens.
    #[tokio::test]
    async fn non_admin_non_owner_is_forbidden() {
        let Some(db) = connect_test_database("anon_acl_forbidden").await else {
            return;
        };
        let owner_id = insert_user(&db).await;
        let stranger_id = insert_user(&db).await;
        let service = catalog_service(&owner_id, /* identity_propagating */ false);
        insert_service(&db, &service).await;
        let service_id = service.id.clone();
        let state = test_app_state(db.clone());

        let list_err = list_anonymous_endpoints(
            State(state.clone()),
            test_auth_user(&stranger_id),
            Path(service_id.clone()),
        )
        .await
        .expect_err("stranger must be forbidden from listing");
        assert!(matches!(list_err, AppError::Forbidden(_)));

        let create_err = create_anonymous_endpoint(
            State(state.clone()),
            test_auth_user(&stranger_id),
            Path(service_id.clone()),
            Json(CreateAnonymousEndpointRequest {
                enabled: true,
                method: "GET".to_string(),
                path_pattern: "/public/**".to_string(),
                daily_quota: 100,
            }),
        )
        .await
        .expect_err("stranger must be forbidden from creating");
        assert!(matches!(create_err, AppError::Forbidden(_)));

        // The forbidden create must not have persisted anything.
        let persisted = load_service(&db, &service_id).await;
        assert!(persisted.anonymous_endpoints.is_empty());
    }

    /// The owner (non-admin creator) is allowed to manage rules on their own
    /// service.
    #[tokio::test]
    async fn owner_can_manage_own_service() {
        let Some(db) = connect_test_database("anon_owner_allowed").await else {
            return;
        };
        let owner_id = insert_user(&db).await;
        let service = catalog_service(&owner_id, /* identity_propagating */ false);
        insert_service(&db, &service).await;
        let service_id = service.id.clone();
        let state = test_app_state(db.clone());

        let _ = create_anonymous_endpoint(
            State(state),
            test_auth_user(&owner_id),
            Path(service_id.clone()),
            Json(CreateAnonymousEndpointRequest {
                enabled: true,
                method: "GET".to_string(),
                path_pattern: "/public/**".to_string(),
                daily_quota: 100,
            }),
        )
        .await
        .expect("owner may create on own service");

        let persisted = load_service(&db, &service_id).await;
        assert_eq!(persisted.anonymous_endpoints.len(), 1);
    }

    /// Deleting an existing rule succeeds and removes it; deleting a missing
    /// rule returns NotFound. Updating a missing rule also returns NotFound.
    #[tokio::test]
    async fn delete_and_update_not_found_paths() {
        let Some(db) = connect_test_database("anon_notfound_paths").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let mut service = catalog_service(&admin_id, /* identity_propagating */ false);
        let rule = AnonymousEndpointRule {
            id: Uuid::new_v4().to_string(),
            enabled: true,
            method: "GET".to_string(),
            path_pattern: "/public/**".to_string(),
            daily_quota: 100,
        };
        service.anonymous_endpoints.push(rule.clone());
        insert_service(&db, &service).await;
        let service_id = service.id.clone();
        let state = test_app_state(db.clone());

        // Updating a non-existent rule -> NotFound.
        let update_err = update_anonymous_endpoint(
            State(state.clone()),
            test_auth_user(&admin_id),
            Path((service_id.clone(), "no-such-rule".to_string())),
            Json(UpdateAnonymousEndpointRequest {
                enabled: Some(false),
                method: None,
                path_pattern: None,
                daily_quota: None,
            }),
        )
        .await
        .expect_err("update of missing rule must be NotFound");
        assert!(matches!(update_err, AppError::NotFound(_)));

        // Deleting the existing rule succeeds and empties the list.
        let remaining = delete_anonymous_endpoint(
            State(state.clone()),
            test_auth_user(&admin_id),
            Path((service_id.clone(), rule.id.clone())),
        )
        .await
        .expect("delete existing rule");
        assert!(remaining.0.endpoints.is_empty());

        let persisted = load_service(&db, &service_id).await;
        assert!(persisted.anonymous_endpoints.is_empty());

        // Deleting again (now missing) -> NotFound.
        let delete_err = delete_anonymous_endpoint(
            State(state),
            test_auth_user(&admin_id),
            Path((service_id, rule.id)),
        )
        .await
        .expect_err("delete of missing rule must be NotFound");
        assert!(matches!(delete_err, AppError::NotFound(_)));
    }

    /// Operating on a non-existent service returns NotFound (service-not-found
    /// path) for both list and create.
    #[tokio::test]
    async fn missing_service_returns_not_found() {
        let Some(db) = connect_test_database("anon_missing_service").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let state = test_app_state(db);

        let list_err = list_anonymous_endpoints(
            State(state.clone()),
            test_auth_user(&admin_id),
            Path("no-such-service".to_string()),
        )
        .await
        .expect_err("list on missing service must be NotFound");
        assert!(matches!(list_err, AppError::NotFound(_)));

        let create_err = create_anonymous_endpoint(
            State(state),
            test_auth_user(&admin_id),
            Path("no-such-service".to_string()),
            Json(CreateAnonymousEndpointRequest {
                enabled: false,
                method: "GET".to_string(),
                path_pattern: "/public/**".to_string(),
                daily_quota: 100,
            }),
        )
        .await
        .expect_err("create on missing service must be NotFound");
        assert!(matches!(create_err, AppError::NotFound(_)));
    }
}
