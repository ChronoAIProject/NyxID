//! HTTP entrance for the reserved namespace. Private routes never become
//! generic proxy targets, approvals, customer usage labels or response headers.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::{AuthMethod, AuthUser},
    services::{platform_model_execution as execution, platform_model_service as registry},
};
use async_trait::async_trait;
use axum::{
    body::Body,
    http::{HeaderMap, Method},
    response::Response,
};
use bson::doc;
use chrono::Utc;
use futures::TryStreamExt;
use tokio_util::sync::CancellationToken;

struct Access {
    state: AppState,
    auth: AuthUser,
    path: String,
    body: bytes::Bytes,
    headers: HeaderMap,
}
impl Access {
    async fn scope(&self) -> AppResult<Option<std::collections::HashSet<String>>> {
        let mut auth = self.auth.clone();
        let person = auth.user_id.to_string();
        if auth.auth_method == AuthMethod::AccessToken {
            let value = self
                .headers
                .get(http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| {
                    v.strip_prefix("Bearer ")
                        .or_else(|| v.strip_prefix("DPoP "))
                })
                .ok_or_else(registry::unavailable)?;
            let claims =
                crate::crypto::jwt::verify_token(&self.state.jwt_keys, &self.state.config, value)?;
            if claims.sub != person {
                return Err(registry::unavailable());
            }
            crate::services::oauth_token_authority_service::validate_claims_for_route(
                &self.state.db,
                &claims,
                "platform_model.gateway",
            )
            .await?;
        }
        if let Some(id) = &auth.api_key_id {
            let key =
                crate::services::key_service::get_api_key(&self.state.db, &person, id).await?;
            if key.expires_at.is_some_and(|e| e <= Utc::now())
                || key.purpose != crate::models::api_key::ApiKeyPurpose::General
            {
                return Err(registry::unavailable());
            }
            let live = crate::mw::auth::api_key_auth_user(
                &self.state.db,
                &key,
                auth.api_key_credential_id.clone(),
                None,
                None,
            )
            .await?;
            live.ensure_llm_proxy_access()?;
            if live.assistant_chat.is_some()
                || live.org_agent_access.is_some()
                || live.execution_permission.is_some()
                // Authentication's enrollment field is only a process-local
                // hint. Cold/stale snapshots cannot authorize this legacy path
                // when the live rollout requires an execution permission.
                || crate::services::execution_permission_enrollment::live_enabled(
                    &self.state.db,
                    &person,
                    None,
                )
                .await?
            {
                return Err(registry::unavailable());
            }
            // Never broaden a request from its authenticated snapshot on refresh.
            auth.allowed_service_ids
                .retain(|id| live.allow_all_services || live.allowed_service_ids.contains(id));
            if auth.allow_all_services {
                auth.allowed_service_ids = live.allowed_service_ids;
            }
            auth.allow_all_services &= live.allow_all_services;
            auth.assistant_operation_scopes = live.assistant_operation_scopes;
            let raw = self
                .headers
                .get(http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .ok_or_else(registry::unavailable)?;
            let (_, live_key, live_credential) =
                crate::services::key_service::validate_api_key(&self.state.db, raw).await?;
            if live_key.id != *id || live_credential != auth.api_key_credential_id {
                return Err(registry::unavailable());
            }
        }
        if let Some(session) = auth.session_id
            && self
                .state
                .db
                .collection::<bson::Document>("sessions")
                .find_one(crate::services::session_authority::authenticated(doc! {
                    "_id": session.to_string(),
                    "user_id": &person,
                    "revoked": false,
                    "expires_at": {"$gt": bson::DateTime::now()},
                }))
                .await?
                .is_none()
        {
            return Err(registry::unavailable());
        }
        // Actor-bound assistant/relay/guest transports land in PR 5. They may
        // not enter this ordinary external gateway through synthetic identities.
        let snapshot = registry::EntitlementSnapshot::load(&self.state.db, &person).await?;
        let catalogs = registry::authorized_catalog_scope(
            &self.state.db,
            &snapshot,
            auth.allow_all_services,
            &auth.allowed_service_ids,
            &auth.assistant_operation_scopes,
        )
        .await?;
        if catalogs.is_none() && self.auth.assistant_operation_scopes.is_empty() {
            return Ok(None);
        }
        let ids = if auth.allow_all_services {
            auth.assistant_operation_scopes
                .keys()
                .cloned()
                .collect::<Vec<_>>()
        } else {
            auth.allowed_service_ids.clone()
        };
        let rows: Vec<crate::models::user_service::UserService> = self
            .state
            .db
            .collection("user_services")
            .find(doc! {"_id":{"$in":ids},"is_active":true,"deleted_at":bson::Bson::Null})
            .await?
            .try_collect()
            .await?;
        let mut allowed = std::collections::HashSet::new();
        let canonical =
            crate::services::proxy_authorization::CanonicalPath::from_mcp_built(&self.path)?;
        for row in rows {
            let Some(catalog) = row.catalog_service_id.as_deref() else {
                continue;
            };
            if catalogs.as_ref().is_some_and(|s| !s.contains(catalog)) {
                continue;
            }
            let exact = registry::authorized_catalog_scope(
                &self.state.db,
                &snapshot,
                false,
                std::slice::from_ref(&row.id),
                &auth.assistant_operation_scopes,
            )
            .await?;
            if exact.as_ref().is_none_or(|s| !s.contains(catalog)) {
                continue;
            }
            let permits = [
                &auth.assistant_operation_scopes,
                &self.auth.assistant_operation_scopes,
            ]
            .into_iter()
            .all(|scopes| {
                crate::services::agent_operation_scope_service::route_visible(
                    scopes,
                    &row.id,
                    Some(catalog),
                    "POST",
                    &self.path,
                ) && crate::services::agent_operation_scope_service::check_inputs(
                    scopes,
                    &row.id,
                    Some(catalog),
                    None,
                    "POST",
                    &canonical,
                    None,
                    &self.body,
                    Some("application/json"),
                )
                .is_ok()
            });
            if permits {
                allowed.insert(catalog.to_owned());
            }
        }
        Ok(Some(allowed))
    }
}
#[async_trait]
impl execution::Authority for Access {
    async fn check(&self, plan: &execution::Plan) -> AppResult<()> {
        registry::require_execution_ready(&self.state.db).await?;
        let model = registry::authorized_model(
            &self.state.db,
            &self.auth.user_id.to_string(),
            &plan.model.public_id,
        )
        .await?;
        if model.revision != plan.model.revision
            || model.routing_revision_id != plan.model.routing_revision_id
        {
            return Err(registry::unavailable());
        }
        if self
            .scope()
            .await?
            .as_ref()
            .is_some_and(|s| !s.contains(&plan.offering.id))
        {
            return Err(registry::unavailable());
        }
        let live = self
            .state
            .db
            .collection::<crate::models::downstream_service::DownstreamService>(
                "downstream_services",
            )
            .find_one(doc! {
                "_id": &plan.offering.id,
                "is_active": true,
                "platform_key.enabled": true,
                "platform_key.purpose": "model_offering",
            })
            .await?
            .ok_or_else(registry::unavailable)?;
        if live.updated_at != plan.offering.updated_at {
            return Err(registry::unavailable());
        }
        let canonical =
            crate::services::proxy_authorization::CanonicalPath::from_mcp_built(&self.path)?;
        let _ = crate::services::proxy_authorization::authorize_proxy_operation(
            &live, "POST", &canonical,
        )?;
        let op = super::llm_gateway::enforce_agent_llm_operations(
            &self.state.db,
            &self.auth,
            None,
            &plan.offering.id,
            "POST",
            &self.path,
            &self.headers,
            None,
            &self.body,
        )
        .await?;
        op.check_outbound(
            &self.auth,
            "POST",
            &self.path,
            None,
            &self.body,
            &self.headers,
        )?;
        super::llm_gateway::preflight_llm_deny_before_resolution(
            &self.state,
            &self.auth,
            &plan.offering.id,
            &self.path,
            "POST",
            Some(&self.body),
        )
        .await
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn execute(
    state: AppState,
    auth: AuthUser,
    path: String,
    method: Method,
    query: Option<String>,
    headers: HeaderMap,
    body: bytes::Bytes,
    value: serde_json::Value,
    cancel: CancellationToken,
) -> AppResult<Response> {
    execute_with_runtime(
        state,
        auth,
        path,
        method,
        query,
        headers,
        body,
        value,
        cancel,
        execution::Runtime::shared(),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_with_runtime(
    state: AppState,
    auth: AuthUser,
    path: String,
    method: Method,
    query: Option<String>,
    headers: HeaderMap,
    body: bytes::Bytes,
    value: serde_json::Value,
    cancel: CancellationToken,
    runtime: std::sync::Arc<execution::Runtime>,
) -> AppResult<Response> {
    if auth.assistant_chat.is_some() {
        return super::assistant_platform_models::execute(
            state, auth, path, method, query, headers, body, value, cancel, runtime,
        )
        .await;
    }
    if !matches!(
        auth.auth_method,
        AuthMethod::Session | AuthMethod::AccessToken | AuthMethod::ApiKey
    ) || auth.assistant_chat.is_some()
        || auth.org_agent_access.is_some()
        // A private route is not a concrete UserService HTTP effect. Until the
        // actor-bound adapter can bind and consume its execution permission,
        // never treat a cached approval as a substitute for that send fence.
        || auth.execution_permission.is_some()
        || auth.api_key_purpose != crate::models::api_key::ApiKeyPurpose::General
        || query.is_some()
        || headers.contains_key(http::header::UPGRADE)
    {
        return Err(registry::unavailable());
    }
    let person = auth.user_id.to_string();
    let public_id = value["model"].as_str().ok_or_else(registry::unavailable)?;
    registry::authorized_model(&state.db, &person, public_id).await?;
    if body.len() > state.config.llm_max_body_size || body.len() > execution::images::MAX_REQUEST {
        return Err(AppError::RequestBodyTooLarge {
            max_bytes: state
                .config
                .llm_max_body_size
                .min(execution::images::MAX_REQUEST),
            context: "Platform model gateway".into(),
        });
    }
    // Check before any decoder reservation/base64 work. Plan and dispatch
    // retain their checks to fence readiness changes after this entrance.
    if execution::images::contains_images(&value) {
        crate::services::security_readiness_service::require_serving(
            &state.db,
            execution::images::READINESS,
        )
        .await?;
    }
    let request = execution::Request::parse_async(method.clone(), path.clone(), value, &person)
        .await?
        .with_ingress_limit(state.config.llm_max_body_size);
    let path = format!(
        "/v1/{}",
        path.trim_start_matches('/')
            .strip_prefix("v1/")
            .unwrap_or(path.trim_start_matches('/'))
    );
    let access = Access {
        state: state.clone(),
        auth: auth.clone(),
        path: path.clone(),
        headers,
        body,
    };
    let scope = access.scope().await?;
    let public_id = request.body["model"]
        .as_str()
        .ok_or_else(registry::unavailable)?;
    let plan = execution::Plan::load(
        &state.db,
        &person,
        public_id,
        scope.as_ref(),
        state.config.billing_rate_cache_ttl_secs,
        &request,
    )
    .await?;
    if request.has_images()
        && access.body.len() as i64 > plan.tariff.tariff().bounds.max_request_bytes
    {
        return Err(AppError::RequestBodyTooLarge {
            max_bytes: plan.tariff.tariff().bounds.max_request_bytes as usize,
            context: "Platform model request".into(),
        });
    }
    execution::Authority::check(&access, &plan).await?;
    // Reuse ordinary approval policy and its bounded notification/wait path.
    super::llm_gateway::check_llm_approval(
        &state,
        &auth,
        &plan.offering.id,
        &plan.offering,
        &path,
        "POST",
        Some(&access.body),
        None,
        false,
        false,
        false,
    )
    .await?;
    let request_id = uuid::Uuid::new_v4().to_string();
    let context = execution::Context {
        db: &state.db,
        keys: &state.encryption_keys,
        billing: &state.billing,
        actor: &person,
        api_key: auth.api_key_id.clone(),
        request_id: request_id.clone(),
        authority: &access,
        cancel: cancel.clone(),
        runtime: &runtime,
    };
    let accepted = execution::dispatch(&context, &plan, &request).await?;
    if !request.streaming {
        let value = execution::consume(&context, &plan, accepted, None).await?;
        return Response::builder()
            .header("content-type", "application/json")
            .header("x-nyxid-request-id", &request_id)
            .body(Body::from(serde_json::to_vec(&value).map_err(|_| {
                AppError::Internal("Model serialization failed".into())
            })?))
            .map_err(|_| AppError::Internal("Model response failed".into()));
    }
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let worker_cancel = cancel.clone();
    let response_id = request_id.clone();
    tokio::spawn(async move {
        let context = execution::Context {
            db: &state.db,
            keys: &state.encryption_keys,
            billing: &state.billing,
            actor: &person,
            api_key: auth.api_key_id.clone(),
            request_id,
            authority: &access,
            cancel: worker_cancel,
            runtime: &runtime,
        };
        if execution::consume(&context, &plan, accepted, Some(&tx))
            .await
            .is_err()
        {
            // Never forward provider bodies, request IDs or destination metadata.
            let _ = tx
                .send(Ok(bytes::Bytes::from_static(
                    b"data: {\"error\":{\"code\":\"model_response_unavailable\",\"message\":\"Model response could not be completed\"}}\n\n",
                )))
                .await;
        }
    });
    let stream = crate::downstream_disconnect::CancelOnDropStream::new(
        tokio_stream::wrappers::ReceiverStream::new(rx),
        cancel,
    );
    Response::builder()
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-store")
        .header("x-nyxid-request-id", response_id)
        .body(Body::from_stream(stream))
        .map_err(|_| AppError::Internal("Model response failed".into()))
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::{
        models::platform_model::*,
        services::platform_model_execution::{adapters::Native, tests::fixture},
        test_utils,
    };
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    pub(crate) async fn enabled() -> (AppState, execution::Plan, execution::Request, AuthUser) {
        enabled_with_readiness(true).await
    }

    async fn enabled_with_readiness(
        ready: bool,
    ) -> (AppState, execution::Plan, execution::Request, AuthUser) {
        let (state, mut plan, request) = fixture(Native::Responses, false).await;
        let actor = "00000000-0000-4000-8000-000000000001";
        state
            .db
            .collection::<crate::models::user::User>("users")
            .insert_one(test_utils::test_user(
                actor,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        if ready {
            crate::services::security_readiness_service::ready_test_replica(&state.db).await;
        }
        let routes = RoutingRevision {
            id: uuid::Uuid::new_v4().to_string(),
            model_id: plan.model.id.clone(),
            model_revision: 1,
            routes: plan.test_routes(),
            created_at: Utc::now(),
        };
        plan.model.routing_revision_id = Some(routes.id.clone());
        state
            .db
            .collection::<RoutingRevision>(ROUTING_COLLECTION)
            .insert_one(routes)
            .await
            .unwrap();
        state
            .db
            .collection::<PlatformModel>(COLLECTION_NAME)
            .insert_one(&plan.model)
            .await
            .unwrap();
        state.db.collection::<bson::Document>("feature_flag_overrides").insert_one(doc!{"_id":uuid::Uuid::new_v4().to_string(),"org_user_id":bson::Bson::Null,"flag_key":registry::FLAG,"target_kind":"global","target_key":bson::Bson::Null,"enabled":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),"updated_by":actor}).await.unwrap();
        let auth = test_utils::test_auth_user(actor);
        (state, plan, request, auth)
    }

    async fn enabled_image(
        ready: bool,
    ) -> (
        AppState,
        execution::Plan,
        execution::Request,
        AuthUser,
        HeaderMap,
    ) {
        use crate::models::api_key::ApiKey;
        let (state, mut plan, _, _) = enabled_with_readiness(ready).await;
        let request = execution::tests::images_tests::image_plan(&mut plan, false);
        state
            .db
            .collection::<PlatformModel>(COLLECTION_NAME)
            .replace_one(doc! {"_id": &plan.model.id}, &plan.model)
            .await
            .unwrap();
        state
            .db
            .collection::<crate::models::platform_model_tariff::ModelTariff>(
                crate::models::platform_model_tariff::COLLECTION_NAME,
            )
            .replace_one(doc! {"_id": &plan.tariff.tariff().id}, plan.tariff.tariff())
            .await
            .unwrap();
        // Isolate the person even when multiple fixture databases run in this
        // process: the production decode limit is keyed by authenticated ID.
        let person = uuid::Uuid::new_v4().to_string();
        state
            .db
            .collection::<crate::models::user::User>("users")
            .insert_one(test_utils::test_user(
                &person,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let raw = "nyxid_ag_image_decode_fixture";
        let key: ApiKey = bson::from_document(doc! {
            "_id": uuid::Uuid::new_v4().to_string(), "user_id": &person,
            "name": "Image decode fixture", "key_prefix": "nyxid_ag_image",
            "key_hash": crate::crypto::token::hash_token(raw), "scopes": "read proxy",
            "is_active": true, "created_at": bson::DateTime::now(),
            "allow_all_services": true,
        })
        .unwrap();
        state
            .db
            .collection::<ApiKey>("api_keys")
            .insert_one(&key)
            .await
            .unwrap();
        let auth = crate::mw::auth::api_key_auth_user(&state.db, &key, None, None, None)
            .await
            .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::AUTHORIZATION,
            format!("Bearer {raw}").parse().unwrap(),
        );
        (state, plan, request, auth, headers)
    }

    #[tokio::test]
    async fn image_approval_wait_does_not_retain_decode_capacity() {
        use crate::models::approval_request::ApprovalRequest;
        let (state, plan, request, auth, headers) = enabled_image(true).await;
        let person = auth.user_id.to_string();
        test_utils::set_test_approval_preference(&state.db, &person, true).await;
        let server = MockServer::start().await;
        let body = request.body.clone();
        let waiting = execute_with_runtime(
            state.clone(),
            auth,
            "chat/completions".into(),
            Method::POST,
            None,
            headers,
            bytes::Bytes::from(serde_json::to_vec(&body).unwrap()),
            body.clone(),
            CancellationToken::new(),
            std::sync::Arc::new(execution::Runtime::mocked(&server.uri())),
        );
        tokio::pin!(waiting);
        let pending = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            tokio::select! {
                result = &mut waiting => panic!("gateway completed before approval; error code: {:?}", result.err().map(|error| error.error_code())),
                pending = async {
                    loop {
                        if let Some(row) = state.db.collection::<ApprovalRequest>("approval_requests")
                            .find_one(doc! {"user_id": &person, "service_id": &plan.offering.id, "status": "pending"})
                            .await.unwrap() {
                            break row;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                } => pending,
            }
        }).await.expect("image request reaches the real human approval wait");
        // The first Request is alive inside the gateway and waiting for a
        // human. The same person can nevertheless validate another image.
        let another =
            execution::Request::parse_async(Method::POST, "chat/completions".into(), body, &person)
                .await
                .unwrap();
        assert!(another.has_images());
        assert_eq!(
            state
                .db
                .collection::<bson::Document>("usage_meter")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        let axum::Json(decision) = crate::handlers::approvals::decide_request(
            axum::extract::State(state.clone()),
            test_utils::test_auth_user(&person),
            axum::extract::Path(pending.id),
            Default::default(),
            HeaderMap::new(),
            axum::Json(crate::handlers::approvals::DecideRequest {
                approved: false,
                duration_sec: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(decision.status, "rejected");
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_secs(10), waiting)
                .await
                .unwrap(),
            Err(AppError::ApprovalFailed { .. })
        ));
    }

    #[tokio::test]
    async fn unready_images_refuse_before_reservation_decode_credentials_or_funding() {
        let (state, plan, request, auth, headers) = enabled_image(false).await;
        let heartbeat =
            crate::services::security_readiness_service::controlled_ready_test_replica(&state.db)
                .await;
        heartbeat.abort();
        let _ = heartbeat.await;
        state
            .db
            .collection::<bson::Document>(crate::models::security_replica::COLLECTION_NAME)
            .update_many(
                doc! {},
                doc! {"$pull": {"capabilities": execution::images::READINESS}},
            )
            .await
            .unwrap();
        registry::authorized_model(&state.db, &auth.user_id.to_string(), &plan.model.public_id)
            .await
            .unwrap();
        state
            .db
            .collection::<bson::Document>("downstream_services")
            .delete_one(doc! {"_id": &plan.test_routes()[0].credential_service_id})
            .await
            .unwrap();
        let server = MockServer::start().await;
        // Reservation before readiness would return 429; decoding before it
        // would reject this invalid raster. Neither may mask the readiness gate.
        let person = auth.user_id.to_string();
        let _occupied = execution::images::hold_decode_for_test(&person);
        let mut body = request.body;
        body["messages"][0]["content"][1]["image_url"]["url"] = json!("data:image/png;base64,AAAA");
        let result = execute_with_runtime(
            state.clone(),
            auth,
            "chat/completions".into(),
            Method::POST,
            None,
            headers,
            bytes::Bytes::from(serde_json::to_vec(&body).unwrap()),
            body,
            CancellationToken::new(),
            std::sync::Arc::new(execution::Runtime::mocked(&server.uri())),
        )
        .await;
        assert!(matches!(
            result,
            Err(AppError::SecurityReadinessUnavailable)
        ));
        assert_eq!(
            state
                .db
                .collection::<bson::Document>("usage_meter")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn gateway_uses_public_model_billing_identity_and_opaque_headers() {
        let (state, _plan, request, auth) = enabled().await;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(Native::Responses.fixture(), "application/json")
                    .insert_header("x-request-id", "private-request")
                    .insert_header("via", "private-region"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let headers = HeaderMap::new();
        let body = bytes::Bytes::from(serde_json::to_vec(&request.body).unwrap());
        let response = execute_with_runtime(
            state,
            auth,
            "chat/completions".into(),
            Method::POST,
            None,
            headers,
            body,
            request.body,
            CancellationToken::new(),
            std::sync::Arc::new(execution::Runtime::mocked(&server.uri())),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), 200);
        assert!(response.headers().get("x-request-id").is_none());
        assert!(response.headers().get("via").is_none());
        assert!(response.headers().get("x-nyxid-request-id").is_some());
        server.verify().await;
    }
    #[tokio::test]
    async fn restricted_general_key_uses_live_grants_without_broadening_its_snapshot() {
        use crate::models::api_key::ApiKey;
        let (state, plan, request, owner) = enabled().await;
        // This fixture tests the key's live service ceiling. Give its owner an
        // explicit approval preference; the ordinary writes-only default must
        // otherwise wait for a human decision for this non-session caller.
        let mut channel = crate::services::notification_service::get_or_create_channel(
            &state.db,
            &owner.user_id.to_string(),
        )
        .await
        .unwrap();
        channel.approval_required = Some(false);
        state
            .db
            .collection::<crate::models::notification_channel::NotificationChannel>(
                "notification_channels",
            )
            .replace_one(doc! {"_id": &channel.id}, channel)
            .await
            .unwrap();
        let connection_id = uuid::Uuid::new_v4().to_string();
        let mut connection = test_utils::test_user_service(
            &connection_id,
            &owner.user_id.to_string(),
            "platform-model",
            &uuid::Uuid::new_v4().to_string(),
            Some(&plan.offering.id),
            None,
        );
        connection.is_active = true;
        state
            .db
            .collection::<crate::models::user_service::UserService>("user_services")
            .insert_one(connection)
            .await
            .unwrap();
        let raw = "nyxid_ag_fixture_platform_model";
        let key: ApiKey = bson::from_document(doc! {
            "_id": uuid::Uuid::new_v4().to_string(),
            "user_id": owner.user_id.to_string(),
            "name": "Restricted model fixture",
            "key_prefix": "nyxid_ag_fixture",
            "key_hash": crate::crypto::token::hash_token(raw),
            "scopes": "read proxy",
            "is_active": true,
            "created_at": bson::DateTime::now(),
            "allow_all_services": false,
            "allowed_service_ids": [&connection_id],
        })
        .unwrap();
        state
            .db
            .collection::<ApiKey>("api_keys")
            .insert_one(&key)
            .await
            .unwrap();
        let auth = crate::mw::auth::api_key_auth_user(&state.db, &key, None, None, None)
            .await
            .unwrap();
        assert!(auth.execution_permission.is_none());
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::AUTHORIZATION,
            format!("Bearer {raw}").parse().unwrap(),
        );
        let access = Access {
            state: state.clone(),
            auth: auth.clone(),
            path: "/v1/chat/completions".into(),
            body: bytes::Bytes::from(serde_json::to_vec(&request.body).unwrap()),
            headers: headers.clone(),
        };
        assert!(
            access
                .scope()
                .await
                .unwrap()
                .unwrap()
                .contains(&plan.offering.id)
        );
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(Native::Responses.fixture(), "application/json"),
            )
            .expect(1)
            .mount(&server)
            .await;
        execute_with_runtime(
            state.clone(),
            auth.clone(),
            "chat/completions".into(),
            Method::POST,
            None,
            headers.clone(),
            access.body.clone(),
            request.body.clone(),
            CancellationToken::new(),
            std::sync::Arc::new(execution::Runtime::mocked(&server.uri())),
        )
        .await
        .unwrap();
        state
            .db
            .collection::<ApiKey>("api_keys")
            .update_one(
                doc! {"_id": &key.id},
                doc! {"$set": {"allowed_service_ids": []}},
            )
            .await
            .unwrap();
        assert!(access.scope().await.unwrap().unwrap().is_empty());
        assert!(
            execute_with_runtime(
                state.clone(),
                auth,
                "chat/completions".into(),
                Method::POST,
                None,
                headers,
                access.body.clone(),
                request.body,
                CancellationToken::new(),
                std::sync::Arc::new(execution::Runtime::mocked(&server.uri())),
            )
            .await
            .is_err()
        );
        // A later live broadening must not widen the request's original snapshot.
        let mut originally_empty = access;
        originally_empty.auth.allowed_service_ids.clear();
        state
            .db
            .collection::<ApiKey>("api_keys")
            .update_one(
                doc! {"_id": &key.id},
                doc! {"$set": {"allowed_service_ids": [&connection_id]}},
            )
            .await
            .unwrap();
        assert!(originally_empty.scope().await.unwrap().unwrap().is_empty());
        // Global enrollment deliberately does not write a person marker. The
        // live resolver must still fence a request with an absent/stale hint.
        originally_empty.auth.allowed_service_ids = vec![connection_id];
        assert!(
            originally_empty
                .scope()
                .await
                .unwrap()
                .unwrap()
                .contains(&plan.offering.id)
        );
        assert!(originally_empty.auth.execution_permission.is_none());
        state
            .db
            .collection::<bson::Document>("feature_flag_overrides")
            .insert_one(doc! {
                "_id": uuid::Uuid::new_v4().to_string(),
                "org_user_id": bson::Bson::Null,
                "flag_key": crate::services::execution_permission_enrollment::FLAG,
                "target_kind": "global",
                "target_key": bson::Bson::Null,
                "enabled": true,
                "created_at": bson::DateTime::now(),
                "updated_at": bson::DateTime::now(),
                "updated_by": owner.user_id.to_string(),
            })
            .await
            .unwrap();
        assert!(
            crate::services::execution_permission_enrollment::live_enabled(
                &state.db,
                &owner.user_id.to_string(),
                None,
            )
            .await
            .unwrap()
        );
        assert!(originally_empty.scope().await.is_err());
        server.verify().await;
    }

    #[tokio::test]
    async fn gateway_default_off_and_unentitled_do_not_disclose_or_send() {
        let (state, plan, request, auth) = enabled().await;
        let server = MockServer::start().await;
        state
            .db
            .collection::<bson::Document>("platform_models")
            .update_one(
                doc! {"_id":&plan.model.id},
                doc! {"$set":{"entitlements.all_people":false}},
            )
            .await
            .unwrap();
        let error = execute_with_runtime(
            state,
            auth,
            "chat/completions".into(),
            Method::POST,
            None,
            HeaderMap::new(),
            bytes::Bytes::from(serde_json::to_vec(&request.body).unwrap()),
            request.body,
            CancellationToken::new(),
            std::sync::Arc::new(execution::Runtime::mocked(&server.uri())),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AppError::NotFound(_)));
        assert!(server.received_requests().await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn live_session_revocation_refuses_after_planning() {
        let (state, plan, request, mut auth) = enabled().await;
        let session = uuid::Uuid::new_v4();
        auth.session_id = Some(session);
        let access = Access {
            state,
            auth,
            path: "/v1/chat/completions".into(),
            body: bytes::Bytes::from(serde_json::to_vec(&request.body).unwrap()),
            headers: HeaderMap::new(),
        };
        assert!(execution::Authority::check(&access, &plan).await.is_err());
    }
    #[tokio::test]
    async fn enrolled_permission_never_bypasses_its_unimplemented_private_route_fence() {
        let (state, _, request, mut auth) = enabled().await;
        auth.execution_permission =
            crate::services::execution_permission_dispatch::RequestContext::enrolled(
                Some(crate::models::execution_permission::VERSION),
                &auth.user_id.to_string(),
                "fixture-key",
                None,
            );
        assert!(auth.execution_permission.is_some());
        let server = MockServer::start().await;
        assert!(
            execute_with_runtime(
                state,
                auth,
                "chat/completions".into(),
                Method::POST,
                None,
                HeaderMap::new(),
                bytes::Bytes::from(serde_json::to_vec(&request.body).unwrap()),
                request.body,
                CancellationToken::new(),
                std::sync::Arc::new(execution::Runtime::mocked(&server.uri()))
            )
            .await
            .is_err()
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }
    #[test]
    fn reserved_gateway_keeps_no_native_model_selector() {
        let body = json!({"model":"nyxid/gpt-5","route":"private","max_tokens":2,"messages":[{"role":"user","content":"hello"}]});
        assert!(execution::Request::parse(&Method::POST, "chat/completions", body).is_err());
    }
}
