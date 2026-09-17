use axum::{
    Json,
    extract::{Path, Query, State},
    http::header,
};

use crate::errors::AppError;
use crate::handlers::admin_helpers::require_admin;
use crate::handlers::admin_service_accounts::{
    require_admin_or_owning_org_admin, resolve_service_account_create_owner,
};
use crate::services::org_service;
use crate::services::{
    options_service::{self, OptionSet, OptionsQuery, OptionsResponse},
    service_account_service,
};
use crate::{AppState, errors::AppResult, mw::auth::AuthUser};

#[utoipa::path(
    get, path = "/api/v1/options/{option_set}",
    params(("option_set" = String, Path, description = "Registered option set: service-scope"), OptionsQuery),
    responses(
        (status = 200, description = "Options authorized for the requested owner", body = OptionsResponse),
        (status = 400, description = "Invalid context or pagination", body = crate::errors::ErrorResponse),
        (status = 403, description = "Service account management access required", body = crate::errors::ErrorResponse),
        (status = 404, description = "Unknown option set or inaccessible account", body = crate::errors::ErrorResponse)
    ),
    security(("bearer_auth" = [])), tag = "Options"
)]
pub async fn get_options(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(name): Path<String>,
    Query(query): Query<OptionsQuery>,
) -> AppResult<(
    [(header::HeaderName, &'static str); 1],
    Json<OptionsResponse>,
)> {
    let option_set = OptionSet::resolve(&name)?;
    query.validate()?;
    let existing = if let Some(id) = &query.service_account_id {
        let sa = service_account_service::get_service_account(&state.db, id).await?;
        // This endpoint requires actual org ownership on the non-global path.
        // Keep the existing management helpers and their callers unchanged.
        if require_admin(&state, &auth).await.is_err() {
            org_service::get_org_user(&state.db, sa.effective_owner_user_id()).await?;
        }
        require_admin_or_owning_org_admin(&state, &auth, &sa).await?;
        if sa.effective_owner_user_id() != query.owner_id {
            return Err(AppError::ValidationError(
                "Service account does not belong to the requested owner".into(),
            ));
        }
        Some(sa)
    } else {
        let actor = auth.user_id.to_string();
        let org = (query.owner_id != actor).then_some(query.owner_id.as_str());
        if let Some(org) = org {
            org_service::get_org_user(&state.db, org).await?;
        }
        resolve_service_account_create_owner(&state, &auth, org).await?;
        None
    };
    let response = options_service::resolve(
        &state.db,
        option_set,
        &query,
        existing.as_ref().map(|sa| sa.allowed_scopes.as_str()),
    )
    .await?;
    Ok((
        [(header::CACHE_CONTROL, "private, no-store")],
        Json(response),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgRole},
        user::{COLLECTION_NAME as USERS, User, UserType},
    };
    use crate::services::{role_service, service_account_scope_service as scopes};
    use crate::test_utils::{
        connect_test_database, test_app_state, test_auth_user, test_membership, test_user,
    };
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use mongodb::bson::doc;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn query(owner: &str) -> OptionsQuery {
        OptionsQuery {
            principal_type: "service_account".into(),
            owner_id: owner.into(),
            service_account_id: None,
            search: None,
            offset: None,
            limit: None,
        }
    }

    #[tokio::test]
    async fn options_acl_owner_isolation_edit_context_and_live_search_pagination() {
        let db = connect_test_database("options_acl")
            .await
            .expect("MongoDB required");
        role_service::seed_system_roles(&db).await.unwrap();
        let roles = role_service::get_platform_role_ids(&db).await.unwrap();
        let actor = Uuid::new_v4().to_string();
        let outsider = Uuid::new_v4().to_string();
        let org = Uuid::new_v4().to_string();
        let mut admin = test_user(&actor, UserType::Person);
        admin.role_ids.push(roles.admin);
        db.collection::<User>(USERS)
            .insert_many([
                admin,
                test_user(&outsider, UserType::Person),
                test_user(&org, UserType::Org),
            ])
            .await
            .unwrap();
        let (own, _) = service_account_service::create_service_account(
            &db,
            "Owner bot",
            None,
            "owner:custom proxy",
            &[],
            None,
            &actor,
        )
        .await
        .unwrap();
        let (team_sa, _) = service_account_service::create_service_account(
            &db,
            "Team bot",
            None,
            "team:custom proxy:* groups",
            &[],
            None,
            &org,
        )
        .await
        .unwrap();
        // Legacy owner fallback and duplicate configured values remain supported.
        db.collection::<mongodb::bson::Document>(crate::models::service_account::COLLECTION_NAME)
            .update_one(doc! { "_id": &own.id }, doc! { "$unset": { "owner_user_id": "" }, "$set": { "allowed_scopes": "owner:custom owner:custom proxy" } }).await.unwrap();
        let state = test_app_state(db.clone());
        let (_, Json(response)) = get_options(
            State(state.clone()),
            test_auth_user(&actor),
            Path("service-scope".into()),
            Query(query(&actor)),
        )
        .await
        .unwrap();
        assert!(response.items.iter().any(|v| v.value == "owner:custom"));
        assert!(!response.items.iter().any(|v| v.value == "team:custom"));
        assert_eq!(
            response.items.iter().filter(|v| v.value == "proxy").count(),
            1
        );
        assert_eq!(
            response
                .items
                .iter()
                .filter(|v| v.value == "owner:custom")
                .count(),
            1
        );
        assert_eq!(
            response
                .items
                .iter()
                .filter(|v| v.source == "backend_definition")
                .count(),
            scopes::DEFINITIONS.len()
        );
        assert!(
            get_options(
                State(state.clone()),
                test_auth_user(&outsider),
                Path("service-scope".into()),
                Query(query(&outsider))
            )
            .await
            .is_err()
        );
        assert!(
            get_options(
                State(state.clone()),
                test_auth_user(&actor),
                Path("service-scope".into()),
                Query(query(&outsider))
            )
            .await
            .is_err()
        );
        assert!(
            get_options(
                State(state.clone()),
                test_auth_user(&outsider),
                Path("service-scope".into()),
                Query(query(&org))
            )
            .await
            .is_err()
        );
        db.collection(MEMBERSHIPS)
            .insert_one(test_membership(&org, &outsider, OrgRole::Admin, None))
            .await
            .unwrap();
        let (_, Json(team)) = get_options(
            State(state.clone()),
            test_auth_user(&outsider),
            Path("service-scope".into()),
            Query(query(&org)),
        )
        .await
        .unwrap();
        assert!(team.items.iter().any(|v| v.value == "team:custom"));
        assert!(!team.items.iter().any(|v| v.value == "owner:custom"));
        let mut page_query = query(&org);
        page_query.limit = Some(1);
        let first = options_service::resolve(&db, OptionSet::ServiceScope, &page_query, None)
            .await
            .unwrap();
        page_query.offset = first.next_offset;
        let second = options_service::resolve(&db, OptionSet::ServiceScope, &page_query, None)
            .await
            .unwrap();
        assert_ne!(first.items[0].value, second.items[0].value);
        assert_eq!(first.version, second.version);
        page_query.search = Some("team:custom".into());
        page_query.offset = None;
        let found = options_service::resolve(
            &db,
            OptionSet::ServiceScope,
            &page_query,
            Some("legacy:read proxy:*"),
        )
        .await
        .unwrap();
        assert_eq!(found.total, 1);
        assert_eq!(found.items[0].value, "team:custom");
        assert_eq!(found.selected_items.len(), 2);
        assert!(
            found
                .selected_items
                .iter()
                .all(|item| !item.disabled && item.source == "configured_scope")
        );
        db.collection::<mongodb::bson::Document>(crate::models::service_account::COLLECTION_NAME)
            .update_one(
                doc! { "_id": &team_sa.id },
                doc! { "$set": { "allowed_scopes": "changed:custom" } },
            )
            .await
            .unwrap();
        let hidden = options_service::resolve(&db, OptionSet::ServiceScope, &page_query, None)
            .await
            .unwrap();
        assert_eq!(hidden.total, 0);
        assert_ne!(hidden.version, first.version);
        let (sa, _) = service_account_service::create_service_account(
            &db,
            "Team bot",
            None,
            "roles",
            &[],
            None,
            &org,
        )
        .await
        .unwrap();
        let mut edit = query(&actor);
        edit.service_account_id = Some(sa.id);
        assert!(
            get_options(
                State(state.clone()),
                test_auth_user(&actor),
                Path("service-scope".into()),
                Query(edit)
            )
            .await
            .is_err()
        );
        assert!(OptionSet::resolve("provider-oauth-scopes").is_err());
        for invalid in [0, 101] {
            let mut q = query(&actor);
            q.limit = Some(invalid);
            assert!(q.validate().is_err());
        }
        let mut q = query(&actor);
        q.principal_type = "user".into();
        assert!(q.validate().is_err());
        let mut q = query("bad-id");
        assert!(q.validate().is_err());
        q.owner_id = actor;
        q.search = Some("x".repeat(201));
        assert!(q.validate().is_err());
    }

    #[tokio::test]
    async fn mounted_options_requires_management_credentials_and_registered_context() {
        let db = connect_test_database("options_routes")
            .await
            .expect("MongoDB required");
        role_service::seed_system_roles(&db).await.unwrap();
        let actor = Uuid::new_v4();
        let mut user = test_user(&actor.to_string(), UserType::Person);
        user.role_ids.push(
            role_service::get_platform_role_ids(&db)
                .await
                .unwrap()
                .admin,
        );
        db.collection::<User>(USERS).insert_one(user).await.unwrap();
        let state = test_app_state(db.clone());
        let token = crate::crypto::jwt::generate_access_token(
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
        let (_, private) = crate::routes::build_router();
        let router = private.with_state(state.clone());
        let path = format!(
            "/api/v1/options/service-scope?principal_type=service_account&owner_id={actor}"
        );
        for (path, token, expected) in [
            (path.clone(), Some(token.clone()), StatusCode::OK),
            (path.clone(), None, StatusCode::UNAUTHORIZED),
            (
                path.replace("service-scope?", "unknown?"),
                Some(token.clone()),
                StatusCode::NOT_FOUND,
            ),
            (
                format!("{path}&unexpected=true"),
                Some(token),
                StatusCode::BAD_REQUEST,
            ),
        ] {
            let mut req = Request::builder().uri(path);
            if let Some(token) = token {
                req = req.header("authorization", format!("Bearer {token}"));
            }
            let response = router
                .clone()
                .oneshot(req.body(Body::empty()).unwrap())
                .await
                .unwrap();
            let status = response.status();
            let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            assert_eq!(status, expected, "{}", String::from_utf8_lossy(&body));
        }
        let (sa, secret) = service_account_service::create_service_account(
            &db,
            "Bot",
            None,
            "proxy",
            &[],
            None,
            &actor.to_string(),
        )
        .await
        .unwrap();
        let token = service_account_service::authenticate_client_credentials(
            &db,
            &state.config,
            &state.jwt_keys,
            &sa.client_id,
            &secret,
            None,
        )
        .await
        .unwrap();
        let response = router
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("authorization", format!("Bearer {}", token.access_token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
