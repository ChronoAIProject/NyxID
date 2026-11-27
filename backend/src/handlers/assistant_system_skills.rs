use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    errors::AppResult,
    models::{assistant_system_skills::AssistantSystemSkills, catalog_skill_revision::SkillPin},
    mw::auth::AuthUser,
    services::{
        agent_skill_service::SkillsResponse, assistant_system_skills_service as system,
        audit_service::AuditActor,
    },
};

#[derive(Debug, Serialize)]
pub struct Response {
    revision: i64,
    skills: Vec<SkillResponse>,
    updated_by: Option<String>,
    updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct SkillResponse {
    skill_id: String,
    name: String,
    version: String,
    sha256: String,
    dependencies: Vec<SkillPin>,
    description: String,
    size_bytes: usize,
}

impl From<Option<AssistantSystemSkills>> for Response {
    fn from(row: Option<AssistantSystemSkills>) -> Self {
        match row {
            None => Self {
                revision: 0,
                skills: vec![],
                updated_by: None,
                updated_at: None,
            },
            Some(row) => Self {
                revision: row.revision,
                updated_by: Some(row.updated_by),
                updated_at: Some(row.updated_at),
                skills: row
                    .skills
                    .into_iter()
                    .map(|s| SkillResponse {
                        skill_id: s.reference.skill_id,
                        name: s.reference.name,
                        version: s.reference.version,
                        sha256: s.reference.sha256,
                        dependencies: s.reference.dependencies,
                        description: s.description,
                        size_bytes: s.size_bytes,
                    })
                    .collect(),
            },
        }
    }
}

pub async fn get(State(state): State<AppState>, auth: AuthUser) -> AppResult<Json<Response>> {
    super::admin_helpers::require_admin(&state, &auth).await?;
    Ok(Json(system::get(&state.db).await?.into()))
}

#[derive(Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    q: String,
    #[serde(default = "first_page")]
    page: u32,
}
#[derive(Deserialize)]
pub struct VersionsQuery {
    #[serde(default = "first_page")]
    page: u32,
}
fn first_page() -> u32 {
    1
}

pub async fn search(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<SearchQuery>,
) -> AppResult<Json<serde_json::Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin(&state, &auth).await?;
    let reader = super::agent_skills::OwnerReader::new(&state, &auth.user_id.to_string());
    Ok(Json(
        crate::services::agent_skill_service::search(&reader, &query.q, query.page).await?,
    ))
}

pub async fn versions(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<VersionsQuery>,
) -> AppResult<Json<serde_json::Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin(&state, &auth).await?;
    let reader = super::agent_skills::OwnerReader::new(&state, &auth.user_id.to_string());
    Ok(Json(
        crate::services::agent_skill_service::versions(&reader, &id, query.page).await?,
    ))
}

pub async fn preview(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, version)): Path<(String, String)>,
) -> AppResult<Json<crate::services::agent_skill_service::Preview>> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin(&state, &auth).await?;
    let reader = super::agent_skills::OwnerReader::new(&state, &auth.user_id.to_string());
    Ok(Json(
        crate::services::agent_skill_service::preview(&reader, &id, &version).await?,
    ))
}

pub async fn set(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(selection): Json<system::Selection>,
) -> AppResult<Json<Response>> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin(&state, &auth).await?;
    let reader = super::agent_skills::OwnerReader::new(&state, &auth.user_id.to_string());
    let row = system::set(
        &state.db,
        &reader,
        &AuditActor::from_auth_user(&auth),
        &selection,
    )
    .await?;
    Ok(Json(Some(row).into()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Toggle {
    #[serde(deserialize_with = "Option::deserialize")]
    enabled: Option<bool>,
}

pub async fn toggle(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, skill_id)): Path<(String, String)>,
    Json(body): Json<Toggle>,
) -> AppResult<Json<SkillsResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    crate::services::assistant_nyxagent::require_enabled(&state.db, &auth.user_id.to_string())
        .await?;
    Ok(Json(
        system::toggle(
            &state.db,
            &AuditActor::from_auth_user(&auth),
            &id,
            &skill_id,
            body.enabled,
        )
        .await?,
    ))
}
