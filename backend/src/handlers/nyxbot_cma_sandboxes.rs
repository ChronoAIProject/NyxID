//! First-party human Sandbox management; model execution uses its own tool gate.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
    services::{
        nyxbot_cma_agent_service as agents,
        nyxbot_cma_gateway::{CatalogCmaGateway, validate_idempotency_key},
        nyxbot_cma_sandbox_service as sandboxes,
    },
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageQuery {
    pub cursor: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabelRequest {
    pub generation: i64,
    pub label: String,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    Pause,
    Revive,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRequest {
    pub generation: i64,
    pub command: Command,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationRequest {
    pub generation: i64,
}
pub(super) async fn owner(state: &AppState, auth: &AuthUser) -> AppResult<String> {
    super::login_client_context::require_first_party_human(auth)?;
    let owner = auth.user_id.to_string();
    agents::require_enabled(&state.db, &owner).await?;
    Ok(owner)
}

fn idempotency_key(headers: &HeaderMap) -> AppResult<&str> {
    let mut values = headers.get_all("idempotency-key").iter();
    let key = values
        .next()
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| AppError::ValidationError("One Idempotency-Key is required".into()))?;
    if values.next().is_some() {
        return Err(AppError::ValidationError(
            "One Idempotency-Key is required".into(),
        ));
    }
    validate_idempotency_key(key)?;
    Ok(key)
}

fn audit(state: &AppState, owner: &str, id: &str, action: &str) {
    crate::services::audit_service::log_async(
        state.db.clone(),
        Some(owner.into()),
        "cma_sandbox_management".into(),
        Some(serde_json::json!({"sandbox_id":id,"action":action})),
        None,
        None,
        None,
        None,
    );
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<PageQuery>,
) -> AppResult<Json<sandboxes::SandboxListDto>> {
    let owner = owner(&state, &auth).await?;
    let gateway = CatalogCmaGateway::new(&state);
    Ok(Json(
        sandboxes::list_live(&state.db, &gateway, &owner, query.cursor.as_deref()).await?,
    ))
}
pub async fn detail(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<sandboxes::SandboxDto>> {
    let owner = owner(&state, &auth).await?;
    let row = sandboxes::get_owned(&state.db, &owner, &id).await?;
    let gateway = CatalogCmaGateway::for_service(&state, &row.cma_service_id);
    Ok(Json(
        sandboxes::detail_live(&state.db, &gateway, &owner, &id).await?,
    ))
}
pub async fn label(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<LabelRequest>,
) -> AppResult<Json<sandboxes::SandboxDto>> {
    let owner = owner(&state, &auth).await?;
    let row = sandboxes::update_label(&state.db, &owner, &id, body.generation, &body.label).await?;
    audit(&state, &owner, &id, "rename");
    Ok(Json(row))
}
pub async fn commands(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CommandRequest>,
) -> AppResult<(StatusCode, Json<sandboxes::SandboxCommandDto>)> {
    let owner = owner(&state, &auth).await?;
    let row = sandboxes::get_owned(&state.db, &owner, &id).await?;
    let gateway = CatalogCmaGateway::for_service(&state, &row.cma_service_id);
    let (command, action) = match body.command {
        Command::Pause => (sandboxes::SandboxCommand::Pause, "pause"),
        Command::Revive => (sandboxes::SandboxCommand::Revive, "revive"),
    };
    let result = sandboxes::command(
        &state.db,
        &gateway,
        &owner,
        &id,
        body.generation,
        command,
        idempotency_key(&headers)?,
    )
    .await?;
    audit(&state, &owner, &id, action);
    Ok((
        if result.pending {
            StatusCode::ACCEPTED
        } else {
            StatusCode::OK
        },
        Json(result),
    ))
}
pub async fn retire(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<GenerationRequest>,
) -> AppResult<(StatusCode, Json<sandboxes::SandboxCommandDto>)> {
    let owner = owner(&state, &auth).await?;
    let row = sandboxes::get_owned(&state.db, &owner, &id).await?;
    let gateway = CatalogCmaGateway::for_service(&state, &row.cma_service_id);
    let result = sandboxes::retire(
        &state.db,
        &gateway,
        &owner,
        &id,
        body.generation,
        idempotency_key(&headers)?,
    )
    .await?;
    audit(&state, &owner, &id, "retire");
    Ok((
        if result.pending {
            StatusCode::ACCEPTED
        } else {
            StatusCode::OK
        },
        Json(result),
    ))
}
pub async fn events(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<PageQuery>,
) -> AppResult<Json<crate::services::nyxbot_cma_gateway::CmaEventSnapshot>> {
    let owner = owner(&state, &auth).await?;
    let row = sandboxes::get_owned(&state.db, &owner, &id).await?;
    let gateway = CatalogCmaGateway::for_service(&state, &row.cma_service_id);
    Ok(Json(
        sandboxes::events(&state.db, &gateway, &owner, &id, query.cursor.as_deref()).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mw::auth::AuthMethod;
    #[tokio::test]
    async fn credential_gate_runs_before_any_database_or_upstream_read() {
        use axum::response::IntoResponse;
        let state = crate::test_utils::test_app_state_no_db().await;
        let mut auth = crate::test_utils::test_auth_user(&uuid::Uuid::new_v4().to_string());
        auth.auth_method = AuthMethod::ApiKey;
        let result = list(
            State(state.clone()),
            auth.clone(),
            Query(PageQuery::default()),
        )
        .await;
        assert_eq!(
            result.unwrap_err().into_response().status(),
            StatusCode::FORBIDDEN
        );
        let result = detail(State(state.clone()), auth.clone(), Path("forged".into())).await;
        assert_eq!(
            result.unwrap_err().into_response().status(),
            StatusCode::FORBIDDEN
        );
        let result = label(
            State(state),
            auth,
            Path("forged".into()),
            Json(LabelRequest {
                generation: 1,
                label: "name".into(),
            }),
        )
        .await;
        assert_eq!(
            result.unwrap_err().into_response().status(),
            StatusCode::FORBIDDEN
        );
    }
    #[test]
    fn only_first_party_human_credentials_reach_management() {
        let mut auth = crate::test_utils::test_auth_user(&uuid::Uuid::new_v4().to_string());
        for method in [
            AuthMethod::Delegated,
            AuthMethod::ApiKey,
            AuthMethod::Relay,
            AuthMethod::ServiceAccount,
        ] {
            auth.auth_method = method;
            assert!(super::super::login_client_context::require_first_party_human(&auth).is_err());
        }
        auth.auth_method = AuthMethod::AccessToken;
        auth.oauth_client_id = Some("third-party".into());
        assert!(super::super::login_client_context::require_first_party_human(&auth).is_err());
        auth.oauth_client_id = None;
        assert!(super::super::login_client_context::require_first_party_human(&auth).is_ok());
        assert!(agents::require_flag(false).is_err());
    }
    #[test]
    fn lifecycle_requires_one_unchanged_idempotency_key_and_strict_body() {
        let mut headers = HeaderMap::new();
        assert!(idempotency_key(&headers).is_err());
        headers.insert("idempotency-key", "original:key".parse().unwrap());
        assert_eq!(idempotency_key(&headers).unwrap(), "original:key");
        headers.append("idempotency-key", "other".parse().unwrap());
        assert!(idempotency_key(&headers).is_err());
        assert!(
            serde_json::from_value::<CommandRequest>(
                serde_json::json!({"generation":1,"command":"pause","service_id":"other"})
            )
            .is_err()
        );
    }
}
