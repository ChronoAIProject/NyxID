use std::time::Duration;

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
    services::{
        audit_service,
        coordination_service::TokenBucketStore,
        monitoring_service::{
            self as monitoring, Access, Client, Configuration, OwnerConfig, ProbeResult,
            WriteConfig,
        },
    },
};

#[derive(Serialize, ToSchema)]
pub struct MonitoringResponse {
    pub service_id: String,
    pub can_manage: bool,
    pub can_probe: bool,
    pub status: ProbeResult,
    pub configuration: Option<Configuration>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RevisionQuery {
    pub revision: i64,
}

async fn access(state: &AppState, auth: &AuthUser, id: &str) -> AppResult<Access> {
    super::login_client_context::require_first_party_human(auth)?;
    monitoring::authorize(&state.db, auth, id).await
}

fn require_manage(access: &Access) -> AppResult<()> {
    if access.can_manage {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "Monitoring configuration requires owner or organization admin access".into(),
        ))
    }
}

async fn admit_check(
    state: &AppState,
    auth: &AuthUser,
    access: &Access,
) -> AppResult<crate::services::cluster_slot_service::RenewableSlotGuard> {
    if !access.can_probe {
        return Err(AppError::Forbidden(
            "This service cannot be checked with the current execution authority".into(),
        ));
    }
    let admission = TokenBucketStore::admit_over_period(
        &state.db,
        "monitoring-check",
        &auth.user_id.to_string(),
        1,
        Duration::from_secs(10),
        3,
    )
    .await?;
    if !admission.allowed {
        return Err(AppError::RateLimitedWithRetry { retry_after: 10 });
    }
    state
        .cluster_slot_manager
        .acquire("monitoring-check", &access.service_id, 1)
        .await?
        .ok_or(AppError::RateLimitedWithRetry { retry_after: 1 })
}

fn audit(state: &AppState, auth: &AuthUser, service_id: &str, action: &str, revision: Option<i64>) {
    audit_service::log_async(
        state.db.clone(),
        Some(auth.user_id.to_string()),
        action.into(),
        Some(serde_json::json!({"user_service_id": service_id, "revision": revision})),
        auth.ip_address.clone(),
        auth.user_agent.clone(),
        auth.api_key_id.clone(),
        auth.api_key_name.clone(),
    );
}

#[utoipa::path(get, path = "/api/v1/keys/{id}/monitoring", params(("id" = String, Path, description = "Exact UserService UUID")), responses((status = 200, body = MonitoringResponse), (status = 503, description = "Monitoring unavailable")), tag = "Keys")]
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<MonitoringResponse>> {
    let access = access(&state, &auth, &id).await?;
    let client = Client::new(&state.config, &state.jwt_keys, &auth, &access, false)?;
    let (status, configuration) = tokio::time::timeout(monitoring::REQUEST_TIMEOUT, async {
        let status = client.status().await?;
        let configuration = if access.can_manage {
            Some(client.configuration().await?)
        } else {
            None
        };
        Ok::<_, AppError>((status, configuration))
    })
    .await
    .map_err(|_| AppError::MonitoringUnavailable)??;
    Ok(Json(MonitoringResponse {
        service_id: access.service_id,
        can_manage: access.can_manage,
        can_probe: access.can_probe,
        status,
        configuration,
    }))
}

#[utoipa::path(put, path = "/api/v1/keys/{id}/monitoring", params(("id" = String, Path)), request_body = WriteConfig, responses((status = 200, body = Configuration), (status = 409, description = "Revision conflict")), tag = "Keys")]
pub async fn save(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<WriteConfig>,
) -> AppResult<Json<Configuration>> {
    let access = access(&state, &auth, &id).await?;
    require_manage(&access)?;
    let result = Client::new(&state.config, &state.jwt_keys, &auth, &access, false)?
        .save(&body)
        .await?;
    audit(
        &state,
        &auth,
        &access.service_id,
        "monitoring_config_updated",
        Some(result.revision),
    );
    Ok(Json(result))
}

#[utoipa::path(delete, path = "/api/v1/keys/{id}/monitoring", params(("id" = String, Path), ("revision" = i64, Query)), responses((status = 200, body = Configuration), (status = 409, description = "Revision conflict")), tag = "Keys")]
pub async fn clear(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<RevisionQuery>,
) -> AppResult<Json<Configuration>> {
    let access = access(&state, &auth, &id).await?;
    require_manage(&access)?;
    let result = Client::new(&state.config, &state.jwt_keys, &auth, &access, false)?
        .clear(query.revision)
        .await?;
    audit(
        &state,
        &auth,
        &access.service_id,
        "monitoring_config_cleared",
        Some(result.revision),
    );
    Ok(Json(result))
}

#[utoipa::path(post, path = "/api/v1/keys/{id}/monitoring/validate", params(("id" = String, Path)), request_body = OwnerConfig, responses((status = 200, body = ProbeResult)), tag = "Keys")]
pub async fn validate(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<OwnerConfig>,
) -> AppResult<Json<ProbeResult>> {
    let access = access(&state, &auth, &id).await?;
    require_manage(&access)?;
    let client = Client::new(&state.config, &state.jwt_keys, &auth, &access, true)?;
    let slot = admit_check(&state, &auth, &access).await?;
    audit(
        &state,
        &auth,
        &access.service_id,
        "monitoring_draft_checked",
        None,
    );
    let result = slot.run_until_lost(client.probe(Some(&body))).await?;
    Ok(Json(result))
}

#[utoipa::path(post, path = "/api/v1/keys/{id}/monitoring/probe", params(("id" = String, Path)), responses((status = 200, body = ProbeResult)), tag = "Keys")]
pub async fn probe(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<ProbeResult>> {
    let access = access(&state, &auth, &id).await?;
    let client = Client::new(&state.config, &state.jwt_keys, &auth, &access, true)?;
    let slot = admit_check(&state, &auth, &access).await?;
    audit(
        &state,
        &auth,
        &access.service_id,
        "monitoring_checked",
        None,
    );
    let result = slot.run_until_lost(client.probe(None)).await?;
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models::{
            org_membership::{
                COLLECTION_NAME as MEMBERSHIPS, MemberScopeSource, OrgMembership, OrgRole,
            },
            user::{COLLECTION_NAME as USERS, User, UserType},
            user_service::{COLLECTION_NAME as SERVICES, UserService},
        },
        test_utils::{
            connect_test_database, test_app_config, test_app_state_with_config, test_auth_user,
            test_user, test_user_service,
        },
    };
    use mongodb::bson::doc;
    use serde_json::{Value, json};
    use uuid::Uuid;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    async fn fixture() -> (AppState, AuthUser, String, MockServer) {
        let db = connect_test_database("monitoring_http").await.unwrap();
        let server = MockServer::start().await;
        let mut config = test_app_config();
        config.chrono_probe_url = Some(server.uri());
        let state = test_app_state_with_config(db, config);
        let user = Uuid::new_v4().to_string();
        let id = Uuid::new_v4().to_string();
        state
            .db
            .collection::<UserService>(SERVICES)
            .insert_one(test_user_service(
                &id,
                &user,
                "duplicate-slug",
                "endpoint",
                None,
                None,
            ))
            .await
            .unwrap();
        (state, test_auth_user(&user), id, server)
    }

    fn status(id: &str) -> Value {
        json!({"service":{"id":id},"health":{"status":"unknown","source":"none","freshness":"unknown"},"readiness":{"status":"unknown","source":"none","freshness":"unknown"}})
    }

    async fn make_member(state: &AppState, auth: &AuthUser, id: &str) {
        let org = Uuid::new_v4().to_string();
        state
            .db
            .collection::<User>(USERS)
            .insert_one(test_user(&org, UserType::Org))
            .await
            .unwrap();
        state
            .db
            .collection::<UserService>(SERVICES)
            .update_one(doc! {"_id":id}, doc! {"$set":{"user_id":&org}})
            .await
            .unwrap();
        state
            .db
            .collection::<OrgMembership>(MEMBERSHIPS)
            .insert_one(OrgMembership {
                id: Uuid::new_v4().to_string(),
                org_user_id: org,
                member_user_id: auth.user_id.to_string(),
                role: OrgRole::Member,
                scope_source: MemberScopeSource::Override,
                allowed_service_ids: Some(vec![id.into()]),
                created_at: chrono::Utc::now(),
                revoked_at: None,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn member_get_and_probe_never_require_manager_config_access() {
        let (state, auth, id, server) = fixture().await;
        make_member(&state, &auth, &id).await;
        let root = format!("/api/v1/monitoring/services/{id}");
        for (verb, suffix) in [("GET", "status"), ("POST", "probes")] {
            Mock::given(method(verb))
                .and(path(format!("{root}/{suffix}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(status(&id)))
                .expect(1)
                .mount(&server)
                .await;
        }
        let response = get(State(state.clone()), auth.clone(), Path(id.clone()))
            .await
            .unwrap()
            .0;
        assert!(!response.can_manage && response.can_probe && response.configuration.is_none());
        assert!(
            save(
                State(state.clone()),
                auth.clone(),
                Path(id.clone()),
                Json(WriteConfig {
                    revision: 0,
                    health: None,
                    readiness: None
                })
            )
            .await
            .is_err()
        );
        assert!(
            validate(
                State(state.clone()),
                auth.clone(),
                Path(id.clone()),
                Json(OwnerConfig::default())
            )
            .await
            .is_err()
        );
        assert!(
            clear(
                State(state.clone()),
                auth.clone(),
                Path(id.clone()),
                Query(RevisionQuery { revision: 0 })
            )
            .await
            .is_err()
        );
        probe(State(state), auth, Path(id)).await.unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|request| !request.url.path().ends_with("/config"))
        );
    }

    #[tokio::test]
    async fn owners_read_exact_config_and_never_join_sibling_slugs() {
        let (state, auth, id, server) = fixture().await;
        let sibling = Uuid::new_v4().to_string();
        state
            .db
            .collection::<UserService>(SERVICES)
            .insert_one(test_user_service(
                &sibling,
                &Uuid::new_v4().to_string(),
                "duplicate-slug",
                "another",
                None,
                None,
            ))
            .await
            .unwrap();
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/monitoring/services/{id}/status")))
            .respond_with(ResponseTemplate::new(200).set_body_json(status(&id)))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/monitoring/services/{id}/config")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"user_service_id":id,"revision":3,"owner_config":{}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let output = get(State(state.clone()), auth.clone(), Path(id))
            .await
            .unwrap()
            .0;
        assert!(output.can_manage);
        assert_eq!(output.configuration.unwrap().revision, 3);
        assert!(
            get(State(state.clone()), auth.clone(), Path(sibling))
                .await
                .is_err()
        );
        assert!(
            get(
                State(state.clone()),
                auth.clone(),
                Path("duplicate-slug".into())
            )
            .await
            .is_err()
        );
        let mut nonhuman = auth;
        nonhuman.auth_method = crate::mw::auth::AuthMethod::ApiKey;
        assert!(
            get(State(state), nonhuman, Path(output.service_id))
                .await
                .is_err()
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn upstream_outage_is_a_monitoring_error_and_does_not_mutate_service() {
        let (state, auth, id, server) = fixture().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503).set_body_string("private upstream failure"))
            .mount(&server)
            .await;
        assert!(matches!(
            get(State(state.clone()), auth, Path(id.clone())).await,
            Err(AppError::MonitoringUnavailable)
        ));
        let service = state
            .db
            .collection::<UserService>(SERVICES)
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap();
        assert!(service.is_active);
        assert_eq!(service.state_version, 1);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn checks_share_actor_rate_limits_and_cluster_service_concurrency() {
        let (state, auth, id, _) = fixture().await;
        let access = access(&state, &auth, &id).await.unwrap();
        let guard = admit_check(&state, &auth, &access).await.unwrap();
        // Another replica's manager sees the same durable service capacity.
        let other = crate::test_utils::test_app_state(state.db.clone());
        assert!(matches!(
            admit_check(&other, &auth, &access).await,
            Err(AppError::RateLimitedWithRetry { retry_after: 1 })
        ));
        drop(guard);
        // The first two attempts consumed 2/3 actor tokens, regardless of replica.
        let other_access = Access {
            service_id: Uuid::new_v4().to_string(),
            can_manage: true,
            can_probe: true,
        };
        let third = admit_check(&other, &auth, &other_access).await.unwrap();
        assert!(matches!(
            admit_check(&state, &auth, &other_access).await,
            Err(AppError::RateLimitedWithRetry { retry_after: 10 })
        ));
        drop(third);
    }
}

#[cfg(test)]
mod callback_tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        routing::get,
    };
    use mongodb::bson::{DateTime, doc};
    use tower::ServiceExt;
    use uuid::Uuid;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn callback_is_bound_to_source_cookie_session_revocation() {
        let db = crate::test_utils::connect_test_database("monitoring_session")
            .await
            .unwrap();
        let server = MockServer::start().await;
        let mut config = crate::test_utils::test_app_config();
        config.chrono_probe_url = Some(server.uri());
        let state = crate::test_utils::test_app_state_with_config(db.clone(), config);
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &user.to_string(),
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let now = chrono::Utc::now();
        db.collection::<mongodb::bson::Document>("sessions").insert_one(doc!{
            "_id":session.to_string(),"user_id":user.to_string(),"purpose":"authenticated","mfa_assured":false,
            "token_hash":"unused-test-hash","revoked":false,"expires_at":DateTime::from_chrono(now+chrono::Duration::hours(1)),
            "created_at":DateTime::from_chrono(now),"last_active_at":DateTime::from_chrono(now),
        }).await.unwrap();
        let mut auth = crate::test_utils::test_auth_user(&user.to_string());
        auth.session_id = Some(session);
        let id = Uuid::new_v4().to_string();
        Mock::given(wiremock::matchers::method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"service":{"id":id},"health":{"status":"unknown"}}),
            ))
            .mount(&server)
            .await;
        let client = Client::new(
            &state.config,
            &state.jwt_keys,
            &auth,
            &Access {
                service_id: id,
                can_manage: true,
                can_probe: true,
            },
            false,
        )
        .unwrap();
        client.status().await.unwrap();
        let requests = server.received_requests().await.unwrap();
        let authorization = requests[0].headers["authorization"]
            .to_str()
            .unwrap()
            .to_string();
        let app = Router::new()
            .route(
                "/api/v1/users/me",
                get(|_: AuthUser| async { StatusCode::OK }),
            )
            .with_state(state);
        let request = || {
            Request::builder()
                .uri("/api/v1/users/me")
                .header("authorization", &authorization)
                .body(Body::empty())
                .unwrap()
        };
        assert_eq!(
            app.clone().oneshot(request()).await.unwrap().status(),
            StatusCode::OK
        );
        db.collection::<mongodb::bson::Document>("sessions")
            .update_one(
                doc! {"_id":session.to_string()},
                doc! {"$set":{"revoked":true}},
            )
            .await
            .unwrap();
        assert_eq!(
            app.oneshot(request()).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
}
