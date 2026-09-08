use axum::{
    Extension, Json,
    body::{Body, to_bytes},
    extract::{Path, State},
    http::Request,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::services::billing::route_inventory::BillingRoutePolicy;
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
    services::{audit_service, codex_connection_service as connection},
    telemetry::TelemetryContext,
};

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = CodexConnectionResponse)]
pub struct ConnectionResponse {
    account_id: String,
    account_email: String,
    provider_id: String,
    provider_slug: &'static str,
    connection: Option<connection::ConnectionVersion>,
    status: &'static str,
    service_id: Option<String>,
    feature: &'static str,
}

async fn response(state: &AppState, user: &AuthUser) -> AppResult<ConnectionResponse> {
    let id = user.user_id.to_string();
    let provider = connection::provider(&state.db).await?;
    let token = connection::current(&state.db, &id, &provider.id).await?;
    let status = match &token {
        Some(token) => connection::connection_status(&state.db, token).await?,
        None => "not_connected",
    };
    Ok(ConnectionResponse {
        account_email: connection::account_email(&state.db, &id).await?,
        account_id: id,
        provider_id: provider.id,
        provider_slug: "openai",
        connection: token.as_ref().map(connection::version),
        status,
        service_id: token
            .as_ref()
            .and_then(|t| t.metadata.as_ref())
            .and_then(|m| m.get("service_id"))
            .cloned(),
        feature: "openai_responses",
    })
}

#[utoipa::path(get, path = "/api/v1/providers/codex-connection",
    responses((status = 200, body = ConnectionResponse), (status = 403, body = crate::errors::ErrorResponse)),
    security(("bearer_auth" = [])), tag = "Codex Connection")]
pub async fn status(
    State(state): State<AppState>,
    user: AuthUser,
) -> AppResult<Json<ConnectionResponse>> {
    super::login_client_context::require_first_party_human(&user)?;
    Ok(Json(response(&state, &user).await?))
}

#[derive(Deserialize, utoipa::ToSchema)]
#[schema(as = CodexConnectionImportBody)]
pub struct ImportBody {
    account_id: String,
    api_key: String,
    expected_connection: Option<connection::ConnectionVersion>,
}

#[tracing::instrument(skip_all)]
#[utoipa::path(post, path = "/api/v1/providers/codex-connection", request_body = ImportBody,
    responses((status = 200, body = ConnectionResponse), (status = 400, body = crate::errors::ErrorResponse),
        (status = 403, body = crate::errors::ErrorResponse), (status = 409, body = crate::errors::ErrorResponse)),
    security(("bearer_auth" = [])), tag = "Codex Connection")]
pub async fn import(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<ImportBody>,
) -> AppResult<Json<ConnectionResponse>> {
    super::login_client_context::require_first_party_human(&user)?;
    let secret = Zeroizing::new(body.api_key);
    if body.account_id != user.user_id.to_string() {
        return Err(AppError::Forbidden(
            "Confirmed account does not match this session".into(),
        ));
    }
    let token = connection::import_api_key(
        &state.db,
        &state.encryption_keys,
        &body.account_id,
        &secret,
        body.expected_connection.as_ref(),
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &user,
        "codex_api_key_imported",
        Some(
            serde_json::json!({"provider_id":token.provider_config_id,"connection_id":token.id,"state_version":token.state_version}),
        ),
    );
    Ok(Json(response(&state, &user).await?))
}

#[derive(Deserialize, utoipa::ToSchema)]
#[schema(as = CodexConnectionVerifyBody)]
pub struct VerifyBody {
    connection: connection::ConnectionVersion,
    model: String,
}

#[tracing::instrument(skip_all)]
#[utoipa::path(post, path = "/api/v1/providers/codex-connection/verify", request_body = VerifyBody,
    responses((status = 200, body = ConnectionResponse), (status = 400, body = crate::errors::ErrorResponse),
        (status = 403, body = crate::errors::ErrorResponse), (status = 409, body = crate::errors::ErrorResponse)),
    security(("bearer_auth" = [])), tag = "Codex Connection")]
pub async fn verify(
    State(state): State<AppState>,
    user: AuthUser,
    Extension(policy): Extension<BillingRoutePolicy>,
    Json(body): Json<VerifyBody>,
) -> AppResult<Json<ConnectionResponse>> {
    super::login_client_context::require_first_party_human(&user)?;
    if body.model.is_empty()
        || body.model.len() > 100
        || !body
            .model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        return Err(AppError::ValidationError(
            "Choose a valid OpenAI model identifier".into(),
        ));
    }
    let id = user.user_id.to_string();
    let token = connection::require_current(&state.db, &id, &body.connection).await?;
    let service_id = connection::registered_service(&state.db, &token).await?;
    let (catalog, _) =
        crate::services::llm_gateway_service::resolve_llm_service_by_slug(&state.db, "openai")
            .await?;
    let binding = connection::verification_binding(&state.db, &token, &service_id).await?;
    let payload = serde_json::json!({"model":body.model,"input":"Reply OK.","max_output_tokens":32,"store":false});
    let request = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/v1/proxy/{}/responses?_nyxid_via={service_id}",
            catalog.id
        ))
        .extension(policy)
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .map_err(|_| AppError::Internal("Verification request construction failed".into()))?;
    let status = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        let response = super::proxy::proxy_request(
            State(state.clone()),
            user.clone(),
            TelemetryContext::default(),
            Path((catalog.id.clone(), "responses".into())),
            request,
        )
        .await;
        match response {
            Ok(response) => verification_result(response).await,
            Err(_) => "saved",
        }
    })
    .await
    .unwrap_or("saved");
    let current = connection::require_current(&state.db, &id, &body.connection).await?;
    if connection::verification_binding(&state.db, &current, &service_id).await? != binding {
        return Err(AppError::Conflict(
            "Credential changed during verification; verify again".into(),
        ));
    }
    connection::record_verification(&state.db, &id, &body.connection, &binding, status).await?;
    Ok(Json(response(&state, &user).await?))
}

async fn verification_result(response: axum::response::Response) -> &'static str {
    let status = response.status();
    if matches!(status.as_u16(), 401 | 403) {
        return "reconnect_required";
    }
    let body = to_bytes(response.into_body(), 256 * 1024).await;
    let Ok(bytes) = body else {
        return "saved";
    };
    let bytes = Zeroizing::new(bytes.to_vec());
    if !status.is_success() {
        return "saved";
    }
    #[derive(Deserialize)]
    struct CompletedResponse<'a> {
        #[serde(borrow)]
        object: &'a str,
        #[serde(borrow)]
        status: &'a str,
        output: Vec<Output<'a>>,
        error: Option<serde::de::IgnoredAny>,
    }
    #[derive(Deserialize)]
    struct Output<'a> {
        #[serde(borrow, rename = "type")]
        kind: &'a str,
        #[serde(borrow)]
        status: Option<&'a str>,
        #[serde(default, borrow)]
        content: Vec<Content<'a>>,
    }
    #[derive(Deserialize)]
    struct Content<'a> {
        #[serde(borrow, rename = "type")]
        kind: &'a str,
        #[serde(borrow)]
        text: Option<std::borrow::Cow<'a, str>>,
    }
    match serde_json::from_slice::<CompletedResponse<'_>>(&bytes) {
        Ok(result)
            if result.object == "response"
                && result.status == "completed"
                && result.error.is_none()
                && result.output.iter().any(|item| {
                    item.kind == "message"
                        && item.status == Some("completed")
                        && item.content.iter().any(|part| {
                            part.kind == "output_text"
                                && part
                                    .text
                                    .as_ref()
                                    .is_some_and(|text| !text.trim().is_empty())
                        })
                }) =>
        {
            "usable"
        }
        _ => "saved",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn verification_requires_complete_bounded_responses_body() {
        for text in ["OK", "OK\n", "\u{4f60}\u{597d}"] {
            let body = serde_json::json!({"object":"response","status":"completed","error":null,
                "output":[{"type":"message","status":"completed","content":[{"type":"output_text","text":text}]}]});
            assert_eq!(
                verification_result(axum::response::Response::new(Body::from(body.to_string())))
                    .await,
                "usable"
            );
        }
        for body in [
            "",
            "{}",
            "{\"object\":\"response\"",
            "{\"object\":\"response\",\"status\":\"completed\",\"output\":[]}",
        ] {
            assert_eq!(
                verification_result(axum::response::Response::new(Body::from(body.to_string())))
                    .await,
                "saved"
            );
        }
        let oversized = Body::from(vec![b'x'; 256 * 1024 + 1]);
        assert_eq!(
            verification_result(axum::response::Response::new(oversized)).await,
            "saved"
        );
        for status in [401, 403] {
            let response = axum::response::Response::builder()
                .status(status)
                .body(Body::from(vec![b'x'; 300_000]))
                .unwrap();
            assert_eq!(verification_result(response).await, "reconnect_required");
        }
        let truncated = Body::from_stream(futures::stream::iter([
            Ok(bytes::Bytes::from_static(b"{")),
            Err(std::io::Error::other("fixture interrupted")),
        ]));
        assert_eq!(
            verification_result(axum::response::Response::new(truncated)).await,
            "saved"
        );
    }
}
