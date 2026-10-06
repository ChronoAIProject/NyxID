use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
    services::{
        assistant_acknowledgement_service as acks, assistant_agent_learning as learning,
        assistant_agent_learning_review as review, assistant_nyxagent as engine,
        assistant_team_service as team,
    },
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionBody {
    #[serde(default)]
    acknowledgement_id: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    revision: Option<i64>,
    #[serde(default)]
    agent_skills_revision: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditBody {
    draft: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigureBody {
    enabled: bool,
    threshold: i64,
}

pub async fn status(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(agent_id): Path<String>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    engine::require_enabled(&state.db, &actor).await?;
    let agent = team::agent(&state.db, &actor, &agent_id).await?;
    crate::services::org_agent_service::require_use(&state.db, &actor, &agent).await?;
    let config = state
        .db
        .collection::<crate::models::assistant_agent_learning::AssistantAgentLearning>(
            crate::models::assistant_agent_learning::CONFIG_COLLECTION_NAME,
        )
        .find_one(mongodb::bson::doc! {"_id": &agent_id})
        .await?;
    Ok(Json(
        json!({"enabled": config.as_ref().is_some_and(|c| c.enabled), "config": config.map(|c| json!({"threshold": c.threshold, "learning_epoch": c.learning_epoch, "config_revision": c.config_revision, "eligible_count": c.eligible_count, "last_run_at": c.last_run_at, "last_success_at": c.last_success_at, "last_error_code": c.last_error_code}))}),
    ))
}

pub async fn configure(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(agent_id): Path<String>,
    Json(body): Json<ConfigureBody>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    engine::require_enabled(&state.db, &actor).await?;
    let row =
        learning::configure(&state.db, &actor, &agent_id, body.enabled, body.threshold).await?;
    Ok(Json(
        json!({"enabled": row.enabled, "threshold": row.threshold, "learning_epoch": row.learning_epoch, "config_revision": row.config_revision}),
    ))
}

pub async fn consent(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(agent_id): Path<String>,
    Json(body): Json<ConfigureBody>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    engine::require_enabled(&state.db, &actor).await?;
    let row = learning::set_member_opt_in(&state.db, &actor, &agent_id, body.enabled).await?;
    Ok(Json(
        json!({"opted_in": row.opted_in, "revision": row.revision, "learning_epoch": row.learning_epoch}),
    ))
}

async fn chat(state: &AppState, owner: &str) -> AppResult<acks::ChatAuthority> {
    let bot = team::ensure_nyxbot(&state.db, owner).await?;
    let home = team::home_thread_for(&state.db, &state.encryption_keys, owner, &bot).await?;
    acks::for_key(&state.db, owner, Some(&home.credential_api_key_id))
        .await?
        .ok_or_else(|| AppError::Forbidden("NyxBot owner card is unavailable".into()))
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(agent_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    Ok(Json(
        json!({"proposals": review::list(&state, &owner, &agent_id, true).await?}),
    ))
}

pub async fn reject(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((agent_id, proposal_id)): Path<(String, String)>,
    Json(body): Json<DecisionBody>,
) -> AppResult<Json<serde_json::Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    review::reject(
        &state,
        &owner,
        &agent_id,
        &proposal_id,
        body.revision.unwrap_or(0),
        body.reason.as_deref().unwrap_or("rejected"),
    )
    .await?;
    Ok(Json(json!({"status":"rejected"})))
}

pub async fn edit(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((agent_id, proposal_id)): Path<(String, String)>,
    Json(body): Json<EditBody>,
) -> AppResult<Json<serde_json::Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let revision = body
        .draft
        .get("revision")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    review::edit(
        &state,
        &owner,
        &agent_id,
        &proposal_id,
        revision,
        &body.draft,
    )
    .await?;
    Ok(Json(json!({"status": "updated"})))
}

pub async fn approve(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((agent_id, proposal_id)): Path<(String, String)>,
    Json(body): Json<DecisionBody>,
) -> AppResult<Json<serde_json::Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let chat = chat(&state, &owner).await?;
    let proposal_row = state
        .db
        .collection::<crate::models::assistant_agent_learning::AssistantAgentLearningProposal>(
            crate::models::assistant_agent_learning::PROPOSALS_COLLECTION_NAME,
        )
        .find_one(mongodb::bson::doc! {"_id": &proposal_id, "agent_id": &agent_id})
        .await?
        .ok_or_else(|| AppError::NotFound("Learning proposal not found".into()))?;
    let expected_revision = body.revision.unwrap_or(proposal_row.revision);
    let expected_skills_revision = body
        .agent_skills_revision
        .unwrap_or(proposal_row.agent_skills_revision);
    let args = review::approval_binding(
        &state,
        &owner,
        &agent_id,
        &proposal_id,
        expected_revision,
        expected_skills_revision,
    )
    .await?;
    if args["agent_id"].as_str() != Some(&agent_id) {
        return Err(AppError::NotFound("Learning proposal not found".into()));
    }
    if let Some(id) = body.acknowledgement_id.as_deref() {
        let _ = id;
    } else {
        let card = acks::request(&state.db, &chat, acks::Request { kind: "action", service: None, tool: Some("nyxid__approve_agent_learning"), arguments: Some(&args), summary: "Publish this reviewed private Ornn skill and attach its exact pinned version to the agent.", platform: false }).await?;
        return Ok(Json(
            json!({"status":"confirmation_required", "acknowledgement":acks::refusal(&card)}),
        ));
    }
    let chat_key = chat.api_key_id.clone();
    let reader = super::agent_skills::Reader {
        state: &state,
        person: &owner,
        thread_key: Some(&chat_key),
        scopes: None,
        chat: Some(std::sync::Arc::new(chat.clone())),
    };
    Ok(Json(
        review::approve(
            &state,
            &chat,
            &agent_id,
            &proposal_id,
            body.acknowledgement_id.as_deref().unwrap_or_default(),
            &args,
            &reader,
        )
        .await?,
    ))
}

pub async fn run_now(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(agent_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    Ok(Json(
        json!({"run_id": learning::run_now(&state, &owner, &agent_id).await?}),
    ))
}
