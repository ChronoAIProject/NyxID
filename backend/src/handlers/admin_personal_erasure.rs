use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::{
    AppState,
    errors::AppResult,
    mw::auth::AuthUser,
    services::personal_erasure_orphan_sweep::jobs::{self, SweepStatus},
};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrphanSweepRequest {
    /// Dry-run is the default. Applying requires the digest returned by a
    /// preceding dry-run so an operator reviews the exact candidate set.
    #[serde(default)]
    pub apply: bool,
    pub digest: Option<String>,
}

/// POST /api/v1/admin/personal-erasure/orphans
pub async fn orphan_sweep(
    State(state): State<AppState>,
    auth: AuthUser,
    request: Option<Json<OrphanSweepRequest>>,
) -> AppResult<(StatusCode, Json<SweepStatus>)> {
    crate::handlers::admin_helpers::require_admin(&state, &auth).await?;
    crate::handlers::login_client_context::require_first_party_human(&auth)?;
    let request = request.map(|Json(value)| value).unwrap_or_default();
    Ok((
        StatusCode::ACCEPTED,
        Json(
            jobs::start(
                &state.db,
                &auth.user_id.to_string(),
                request.apply,
                request.digest.as_deref(),
            )
            .await?,
        ),
    ))
}

/// GET /api/v1/admin/personal-erasure/orphans/{job_id}
pub async fn orphan_sweep_status(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<SweepStatus>> {
    crate::handlers::admin_helpers::require_admin(&state, &auth).await?;
    crate::handlers::login_client_context::require_first_party_human(&auth)?;
    Ok(Json(jobs::status(&state.db, &id).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        crypto::{jwt, token::hash_token},
        models::user::{User, UserType},
        services::{role_service, service_account_service},
        test_utils::{connect_transaction_test_database, test_app_state, test_user},
    };
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
    };
    use mongodb::bson::{Document, doc};
    use tower::ServiceExt;
    use uuid::Uuid;

    struct Fixture {
        state: AppState,
        app: Router,
        admin: String,
        admin_token: String,
    }

    impl Fixture {
        async fn new(label: &str) -> Self {
            let db = connect_transaction_test_database(label).await;
            crate::test_utils::init_test_audit_chain_hmac_key();
            role_service::seed_system_roles(&db).await.unwrap();
            let admin = Uuid::new_v4().to_string();
            let mut user = test_user(&admin, UserType::Person);
            user.role_ids.push(
                role_service::get_platform_role_ids(&db)
                    .await
                    .unwrap()
                    .admin,
            );
            db.collection::<User>(crate::models::user::COLLECTION_NAME)
                .insert_one(user)
                .await
                .unwrap();
            let state = test_app_state(db);
            let admin_token = jwt::generate_access_token(
                &state.jwt_keys,
                &state.config,
                &Uuid::parse_str(&admin).unwrap(),
                "openid account:read",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let (public, private) = crate::routes::build_router_with_state(state.clone());
            Self {
                state: state.clone(),
                app: public.merge(private).with_state(state),
                admin,
                admin_token,
            }
        }

        async fn request(&self, headers: &[(&str, String)]) -> StatusCode {
            let mut request = Request::post("/api/v1/admin/personal-erasure/orphans")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap();
            for (name, value) in headers {
                request.headers_mut().insert(
                    axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                    value.parse().unwrap(),
                );
            }
            self.app.clone().oneshot(request).await.unwrap().status()
        }
    }

    fn bearer(token: &str) -> (&'static str, String) {
        ("authorization", format!("Bearer {token}"))
    }

    #[tokio::test]
    async fn orphan_sweep_http_starts_and_polls_persisted_job() {
        let fixture = Fixture::new("orphan_sweep_http_poll").await;
        let response = fixture
            .app
            .clone()
            .oneshot(
                Request::post("/api/v1/admin/personal-erasure/orphans")
                    .header("authorization", format!("Bearer {}", fixture.admin_token))
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let started: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(started["status"], "pending");
        let id = started["job_id"].as_str().unwrap();
        crate::services::personal_erasure_orphan_sweep::jobs::finish(&fixture.state.db, id)
            .await
            .unwrap();
        let path = format!("/api/v1/admin/personal-erasure/orphans/{id}");
        let polled = fixture
            .app
            .clone()
            .oneshot(
                Request::get(&path)
                    .header("authorization", format!("Bearer {}", fixture.admin_token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(polled.status(), StatusCode::OK);
        let body = axum::body::to_bytes(polled.into_body(), 64 * 1024)
            .await
            .unwrap();
        let done: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(done["status"], "complete");
        assert_eq!(done["report"]["candidate_count"], 0);
        assert_eq!(
            fixture
                .app
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        fixture.state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn orphan_sweep_http_requires_first_party_admin_human() {
        let fixture = Fixture::new("orphan_sweep_http_auth").await;
        assert_eq!(
            fixture.request(&[bearer(&fixture.admin_token)]).await,
            StatusCode::ACCEPTED
        );

        let plain = Uuid::new_v4().to_string();
        fixture
            .state
            .db
            .collection::<User>(crate::models::user::COLLECTION_NAME)
            .insert_one(test_user(&plain, UserType::Person))
            .await
            .unwrap();
        let plain_token = jwt::generate_access_token(
            &fixture.state.jwt_keys,
            &fixture.state.config,
            &Uuid::parse_str(&plain).unwrap(),
            "openid account:read",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            fixture.request(&[bearer(&plain_token)]).await,
            StatusCode::FORBIDDEN
        );

        let raw_key = format!("nyxid_{}", Uuid::new_v4().simple());
        let key_id = Uuid::new_v4().to_string();
        fixture
            .state
            .db
            .collection::<Document>(crate::models::api_key::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": &key_id,
                "user_id": &fixture.admin,
                "name": "orphan sweep test key",
                "key_prefix": "nyxid_test",
                "key_hash": hash_token(&raw_key),
                "scopes": "account:read",
                "is_active": true,
                "created_at": mongodb::bson::DateTime::now(),
            })
            .await
            .unwrap();
        assert_eq!(
            fixture.request(&[("x-api-key", raw_key)]).await,
            StatusCode::FORBIDDEN
        );

        let owner = Uuid::parse_str(&fixture.admin).unwrap();
        let delegated = jwt::generate_delegated_access_token(
            &fixture.state.jwt_keys,
            &fixture.state.config,
            &owner,
            "account:read",
            "orphan-sweep-client",
            60,
            None,
        )
        .unwrap();
        assert_eq!(
            fixture.request(&[bearer(&delegated)]).await,
            StatusCode::FORBIDDEN
        );

        let relay = jwt::generate_relay_access_token(
            &fixture.state.jwt_keys,
            &fixture.state.config,
            &owner,
            "account:read",
            None,
            &jwt::RelayAgentScope {
                api_key_id: key_id,
                api_key_name: "orphan sweep test key".into(),
                allowed_service_ids: vec![],
                allowed_node_ids: vec![],
                allow_all_services: true,
                allow_all_nodes: true,
            },
        )
        .unwrap();
        assert_eq!(
            fixture.request(&[bearer(&relay)]).await,
            StatusCode::FORBIDDEN
        );

        let oauth = jwt::generate_oauth_access_token(
            &fixture.state.jwt_keys,
            &fixture.state.config,
            &owner,
            "account:read",
            None,
            None,
            None,
            None,
            None,
            "orphan-sweep-oauth-client",
        )
        .unwrap();
        assert!(matches!(
            fixture.request(&[bearer(&oauth)]).await,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));

        let (service_account, secret) = service_account_service::create_service_account(
            &fixture.state.db,
            "orphan sweep service account",
            None,
            "account:read",
            &[],
            None,
            &fixture.admin,
        )
        .await
        .unwrap();
        let service_token = service_account_service::authenticate_client_credentials(
            &fixture.state.db,
            &fixture.state.config,
            &fixture.state.jwt_keys,
            &service_account.client_id,
            &secret,
            Some("account:read"),
        )
        .await
        .unwrap()
        .access_token;
        assert!(matches!(
            fixture.request(&[bearer(&service_token)]).await,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));
    }
}
