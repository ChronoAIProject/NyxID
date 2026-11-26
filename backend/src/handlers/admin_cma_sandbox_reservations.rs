//! First-party platform-admin access to held CMA account-slot reservations.

use super::admin_helpers::require_admin;
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
    services::{
        assistant_cma_gateway::CatalogCmaGateway, assistant_cma_reservation_service as reservations,
    },
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub owner_id: Option<String>,
    pub states: Option<String>,
    pub after: Option<String>,
    pub limit: Option<i64>,
}

#[derive(serde::Serialize)]
pub struct ListResponse {
    pub items: Vec<reservations::ReservationRow>,
    pub next_cursor: Option<String>,
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<ListResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    require_admin(&state, &auth).await?;
    let limit = query
        .limit
        .unwrap_or(50)
        .clamp(1, reservations::MAX_PAGE_SIZE);
    if query.states.as_ref().is_some_and(|value| value.len() > 128) {
        return Err(AppError::ValidationError(
            "Invalid reservation states".into(),
        ));
    }
    let states = query.states.map(|value| {
        value
            .split(',')
            .filter(|state| !state.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    });
    let items = reservations::list(
        &state.db,
        query.owner_id.as_deref(),
        states.as_deref(),
        query.after.as_deref(),
        limit,
    )
    .await?;
    let next_cursor = if items.len() == limit as usize {
        items.last().map(|item| item.id.clone())
    } else {
        None
    };
    Ok(Json(ListResponse { items, next_cursor }))
}

pub async fn inspect(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<reservations::Inspection>> {
    super::login_client_context::require_first_party_human(&auth)?;
    require_admin(&state, &auth).await?;
    let row = reservations::get(&state.db, &id).await?;
    let gateway = CatalogCmaGateway::for_service(&state, &row.cma_service_id);
    let actor = crate::services::audit_service::AuditActor::from_auth_user(&auth);
    Ok(Json(
        reservations::inspect(&state.db, &gateway, &id, &actor).await?,
    ))
}

#[derive(serde::Serialize)]
pub struct ResolveResponse {
    pub reservation: reservations::ReservationRow,
}

pub async fn resolve(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<reservations::ResolveRequest>,
) -> AppResult<(StatusCode, Json<ResolveResponse>)> {
    super::login_client_context::require_first_party_human(&auth)?;
    require_admin(&state, &auth).await?;
    let row = reservations::get(&state.db, &id).await?;
    let gateway = CatalogCmaGateway::for_service(&state, &row.cma_service_id);
    let actor = crate::services::audit_service::AuditActor::from_auth_user(&auth);
    let result = reservations::resolve(&state.db, &gateway, &id, &body, &actor).await;
    if let Err(error) = &result {
        // Fixed metadata only, never caller-supplied evidence or provider text.
        crate::services::audit_service::log_for_user(
            state.db.clone(),
            &auth,
            "admin_cma_sandbox_reservation_resolved",
            Some(serde_json::json!({
                "reservation_id":row.id,"owner_id":row.user_id,"cma_service_id":row.cma_service_id,
                "action":body.action.as_str(),
                "result":"refused","reason":match error { AppError::Conflict(_)=>"state_or_evidence_conflict",AppError::Forbidden(_)=>"authority_refused",_=>"resolution_unavailable" },
            })),
        );
    }
    Ok((
        StatusCode::OK,
        Json(ResolveResponse {
            reservation: result?,
        }),
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn resolve_requires_observed_revisions_and_rejects_caller_proof() {
        use super::reservations::ResolveRequest;
        assert!(serde_json::from_value::<ResolveRequest>(serde_json::json!({})).is_err());
        let body = serde_json::json!({"generation":1,"revision":0,"creation_revision":2,"action":"release"});
        assert!(serde_json::from_value::<ResolveRequest>(body.clone()).is_ok());
        let mut forged = body;
        forged["evidence"] = serde_json::json!("trust_confirmation_required");
        assert!(serde_json::from_value::<ResolveRequest>(forged).is_err());
    }
}

#[cfg(test)]
mod authorization_tests {
    use crate::{
        models::user::{COLLECTION_NAME as USERS, User, UserType},
        services::role_service,
    };
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use base64::Engine;
    use mongodb::bson::doc;
    use serde_json::json;
    use tower::ServiceExt;

    #[tokio::test]
    async fn mounted_admin_routes_require_first_party_human_admins() {
        Box::pin(check_routes()).await;
    }

    async fn check_routes() {
        let db = crate::test_utils::connect_transaction_test_database("cma_reservation_auth").await;
        let state = crate::test_utils::test_app_state(db.clone());
        role_service::seed_system_roles(&db).await.unwrap();
        let roles = role_service::get_platform_role_ids(&db).await.unwrap();
        let actor = uuid::Uuid::new_v4();
        let mut user = crate::test_utils::test_user(&actor.to_string(), UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .unwrap();
        let human = crate::crypto::jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &actor,
            "openid profile account:read",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let now = chrono::Utc::now();
        db.collection::<crate::models::oauth_client::OauthClient>(
            crate::models::oauth_client::COLLECTION_NAME,
        )
        .insert_one(crate::models::oauth_client::OauthClient {
            id: "private-client-id".into(),
            client_name: "Admin reservation authority fixture".into(),
            client_secret_hash: String::new(),
            redirect_uris: vec![],
            allowed_scopes: "openid profile".into(),
            scope_provenance: Default::default(),
            grant_types: "authorization_code".into(),
            client_type: "public".into(),
            is_active: true,
            delegation_scopes: String::new(),
            default_service_catalog_slugs: vec![],
            broker_capability_enabled: false,
            first_party_management: false,
            revocation_webhook_url: None,
            revocation_webhook_secret_encrypted: None,
            connection_webhook_url: None,
            connection_webhook_secret_encrypted: None,
            connection_webhook_key_id: None,
            secret_context_versions: Default::default(),
            connection_webhook_enabled: false,
            created_by: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
        let consent = crate::services::consent_service::grant_consent_with_services(
            &db,
            &actor.to_string(),
            "private-client-id",
            "openid profile",
            None,
        )
        .await
        .unwrap();
        let reference = crate::services::consent_service::ensure_issuance_reference(&db, &consent)
            .await
            .unwrap();
        let oauth = crate::crypto::jwt::generate_oauth_access_token_with_consent(
            &state.jwt_keys,
            &state.config,
            &actor,
            "openid profile",
            None,
            None,
            None,
            None,
            None,
            "private-client-id",
            Some(&reference),
        )
        .unwrap();
        let api_key = crate::services::key_service::create_api_key(
            &db,
            &actor.to_string(),
            "Admin reservation authorization",
            "read write admin openid profile email services:read services:write proxy",
            None,
            None,
            None,
            None,
            Some(true),
            None,
            Some(true),
            None,
            None,
            Some("codex"),
            None,
        )
        .await
        .unwrap();
        let (key_owner, active_key, _) =
            crate::services::key_service::observe_api_key(&db, &api_key.full_key)
                .await
                .unwrap();
        assert_eq!(key_owner, actor.to_string());
        assert_eq!(
            active_key.scopes,
            "read write admin openid profile email services:read services:write proxy"
        );
        let (_, router) = crate::routes::build_router();
        let app = router.with_state(state);
        let paths = [
            ("GET", "/api/v1/admin/cma-sandbox-reservations"),
            (
                "POST",
                "/api/v1/admin/cma-sandbox-reservations/missing/inspect",
            ),
            (
                "POST",
                "/api/v1/admin/cma-sandbox-reservations/missing/resolve",
            ),
        ];
        let body = json!({"generation":1,"revision":0,"creation_revision":0,"action":"release"})
            .to_string();
        for (method, path) in paths {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .header("authorization", format!("Bearer {human}"))
                        .header("content-type", "application/json")
                        .body(Body::from(body.clone()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "non-admin {path}");
        }
        user.role_ids.push(roles.admin);
        db.collection::<User>(USERS)
            .replace_one(doc! {"_id":&user.id}, user)
            .await
            .unwrap();
        for (method, path) in paths {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .header("authorization", format!("Bearer {human}"))
                        .header("content-type", "application/json")
                        .body(Body::from(body.clone()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                if method == "GET" {
                    StatusCode::OK
                } else {
                    StatusCode::NOT_FOUND
                },
                "admin {path}"
            );
            // The real router's deny-only middleware rejects these credential
            // classes before permitting a delegated admin's account:read GET.
            for claims in [
                json!({"relay":true}),
                json!({"sa":true}),
                json!({"delegated":true,"scope":"account:read"}),
            ] {
                let token = format!(
                    "header.{}.signature",
                    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string())
                );
                let response = app
                    .clone()
                    .oneshot(
                        Request::builder()
                            .method(method)
                            .uri(path)
                            .header("authorization", format!("Bearer {token}"))
                            .header("content-type", "application/json")
                            .body(Body::from(body.clone()))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::FORBIDDEN, "{claims} {path}");
            }
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .header("authorization", format!("Bearer {oauth}"))
                        .header("content-type", "application/json")
                        .body(Body::from(body.clone()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "OAuth client {path}"
            );
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .header("x-api-key", &api_key.full_key)
                        .header("content-type", "application/json")
                        .body(Body::from(body.clone()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "API key {path}");
        }
        db.drop().await.unwrap();
    }
}
