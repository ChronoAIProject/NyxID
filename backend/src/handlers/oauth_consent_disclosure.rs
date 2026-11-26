use axum::{Json, extract::State, http::header, response::IntoResponse};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::{AuthUser, require_first_party_human},
    services::{
        oauth_consent_disclosure::{self, CredentialKind, OperationCounts, UpstreamScope},
        user_service_service::CredentialSource,
    },
};

const MAX_SERVICES: usize = 100;

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConsentDisclosureRequest {
    #[schema(max_items = 100)]
    service_ids: Vec<String>,
}

impl ConsentDisclosureRequest {
    fn validate(&self) -> AppResult<()> {
        if self.service_ids.len() > MAX_SERVICES {
            return Err(AppError::ValidationError(
                "At most 100 service IDs may be requested".into(),
            ));
        }
        if self
            .service_ids
            .iter()
            .any(|id| id.len() != 36 || Uuid::parse_str(id).is_err())
        {
            return Err(AppError::ValidationError(
                "Service IDs must be hyphenated UUIDs".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Serialize, ToSchema)]
pub struct ConsentDisclosureResponse {
    access_token_ttl_secs: i64,
    broker_access_token_ttl_secs: i64,
    refresh_token_ttl_secs: i64,
    services: Vec<ServiceDisclosureResponse>,
}

#[derive(Serialize, ToSchema)]
struct ServiceDisclosureResponse {
    service_id: String,
    credential_source: CredentialSourceResponse,
    credential_kind: CredentialKind,
    upstream_scopes: Option<Vec<UpstreamScope>>,
    known_operations: Option<KnownOperationsResponse>,
    capabilities: CapabilitiesResponse,
}

#[derive(Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CredentialSourceResponse {
    Personal,
    Org { org_name: String },
}

impl From<CredentialSource> for CredentialSourceResponse {
    fn from(source: CredentialSource) -> Self {
        match source {
            CredentialSource::Personal => Self::Personal,
            CredentialSource::Org { org_name, .. } => Self::Org { org_name },
        }
    }
}

#[derive(Serialize, ToSchema)]
struct KnownOperationsResponse {
    read: u32,
    write: u32,
    delete: u32,
    unclassified: u32,
    limiting: bool,
}

impl From<OperationCounts> for KnownOperationsResponse {
    fn from(counts: OperationCounts) -> Self {
        Self {
            read: counts.read,
            write: counts.write,
            delete: counts.delete,
            unclassified: counts.unclassified,
            limiting: false,
        }
    }
}

#[derive(Serialize, ToSchema)]
struct CapabilitiesResponse {
    model_calls: Option<bool>,
    binary_upload: Option<bool>,
}

#[utoipa::path(
    post,
    path = "/api/v1/user-services/consent-disclosure",
    request_body = ConsentDisclosureRequest,
    responses(
        (status = 200, description = "Read-only consent display metadata", body = ConsentDisclosureResponse),
        (status = 400, description = "Invalid service IDs", body = crate::errors::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 403, description = "First-party human account required", body = crate::errors::ErrorResponse),
    ),
    security(("bearer_auth" = [])),
    tag = "User Services"
)]
pub async fn read(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ConsentDisclosureRequest>,
) -> AppResult<impl IntoResponse> {
    require_first_party_human(&auth)?;
    body.validate()?;
    let services =
        oauth_consent_disclosure::list(&state.db, &auth.user_id.to_string(), &body.service_ids)
            .await?
            .into_iter()
            .map(|row| ServiceDisclosureResponse {
                service_id: row.service_id,
                credential_source: row.source.into(),
                credential_kind: row.credential_kind,
                upstream_scopes: row.upstream_scopes,
                known_operations: row.known_operations.map(Into::into),
                capabilities: CapabilitiesResponse {
                    model_calls: row.model_calls,
                    binary_upload: row.binary_upload,
                },
            })
            .collect();
    Ok((
        [(header::CACHE_CONTROL, "private, no-store")],
        Json(ConsentDisclosureResponse {
            access_token_ttl_secs: state.config.jwt_access_ttl_secs,
            broker_access_token_ttl_secs:
                crate::services::oauth_broker_service::BROKER_ACCESS_TTL_SECS,
            refresh_token_ttl_secs: state.config.jwt_refresh_ttl_secs,
            services,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mw::auth::AuthMethod, test_utils};

    #[test]
    fn consent_disclosure_validates_batch_size_and_ids() {
        let id = Uuid::new_v4().to_string();
        assert!(
            ConsentDisclosureRequest {
                service_ids: vec![id.clone(); 100]
            }
            .validate()
            .is_ok()
        );
        assert!(
            ConsentDisclosureRequest {
                service_ids: vec![id; 101]
            }
            .validate()
            .is_err()
        );
        assert!(
            ConsentDisclosureRequest {
                service_ids: vec!["catalog-slug".into()]
            }
            .validate()
            .is_err()
        );
    }

    #[tokio::test]
    async fn consent_disclosure_requires_first_party_human_and_reports_configured_lifetimes() {
        let mut state = test_utils::test_app_state_no_db().await;
        let actor = test_utils::test_auth_user(&Uuid::new_v4().to_string());
        for (method, client) in [
            (AuthMethod::ApiKey, None),
            (AuthMethod::Delegated, None),
            (AuthMethod::Relay, None),
            (AuthMethod::ServiceAccount, None),
            (AuthMethod::AccessToken, Some("oauth-app".to_owned())),
        ] {
            let mut caller = actor.clone();
            caller.auth_method = method;
            caller.oauth_client_id = client;
            let result = read(
                State(state.clone()),
                caller,
                Json(ConsentDisclosureRequest {
                    service_ids: vec![],
                }),
            )
            .await;
            assert_eq!(
                result
                    .err()
                    .expect("unsupported credential")
                    .into_response()
                    .status(),
                403
            );
        }
        state.config.jwt_access_ttl_secs = 1200;
        state.config.jwt_refresh_ttl_secs = 86400;
        let response = read(
            State(state),
            actor,
            Json(ConsentDisclosureRequest {
                service_ids: vec![],
            }),
        )
        .await
        .expect("empty disclosure avoids database access")
        .into_response();
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store"
        );
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["access_token_ttl_secs"], 1200);
        assert_eq!(payload["refresh_token_ttl_secs"], 86400);
        assert_eq!(payload["services"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn consent_disclosure_is_mounted_on_the_public_api_router() {
        use crate::{
            models::user::{self, User, UserType},
            services::token_service,
        };
        use axum::{
            body::Body,
            http::{Request, StatusCode},
        };
        use tower::ServiceExt;

        let Some(db) = test_utils::connect_test_database("oauth_consent_disclosure_route").await
        else {
            return;
        };
        let actor = Uuid::new_v4().to_string();
        db.collection::<User>(user::COLLECTION_NAME)
            .insert_one(test_utils::test_user(&actor, UserType::Person))
            .await
            .unwrap();
        let session = token_service::create_session(&db, &actor, None, None)
            .await
            .unwrap();
        let state = test_utils::test_app_state(db);
        let (public, private) = crate::routes::build_router_with_state(state.clone());
        let app = public.merge(private).with_state(state);
        for cookie in [None, Some(format!("nyx_session={}", session.session_token))] {
            let mut request = Request::post("/api/v1/user-services/consent-disclosure")
                .header(header::CONTENT_TYPE, "application/json");
            if let Some(cookie) = cookie.as_ref() {
                request = request.header(header::COOKIE, cookie.as_str());
            }
            let response = Box::pin(
                app.clone()
                    .oneshot(request.body(Body::from(r#"{"service_ids":[]}"#)).unwrap()),
            )
            .await
            .unwrap();
            if cookie.is_some() {
                assert_eq!(response.status(), StatusCode::OK);
                assert_eq!(
                    response.headers()[header::CACHE_CONTROL],
                    "private, no-store"
                );
            } else {
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            }
        }
    }
}
