use crate::{
    AppState,
    errors::AppResult,
    models::platform_help as model,
    mw::auth::AuthUser,
    services::{audit_service, platform_help_service as service},
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, header},
};
use serde::{Deserialize, Serialize};
fn headers() -> HeaderMap {
    HeaderMap::from_iter([(header::CACHE_CONTROL, "no-store".parse().unwrap())])
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Article {
    id: String,
    title: String,
    body: String,
    #[serde(default)]
    tags: Vec<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UseCase {
    id: String,
    title: String,
    description: String,
    example_prompts: Vec<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Content {
    articles: Vec<Article>,
    use_cases: Vec<UseCase>,
}
impl From<model::HelpContent> for Content {
    fn from(c: model::HelpContent) -> Self {
        Self {
            articles: c
                .articles
                .into_iter()
                .map(|a| Article {
                    id: a.id,
                    title: a.title,
                    body: a.body,
                    tags: a.tags,
                })
                .collect(),
            use_cases: c
                .use_cases
                .into_iter()
                .map(|a| UseCase {
                    id: a.id,
                    title: a.title,
                    description: a.description,
                    example_prompts: a.example_prompts,
                })
                .collect(),
        }
    }
}
impl From<Content> for model::HelpContent {
    fn from(c: Content) -> Self {
        Self {
            articles: c
                .articles
                .into_iter()
                .map(|a| model::HelpArticle {
                    id: a.id,
                    title: a.title,
                    body: a.body,
                    tags: a.tags,
                })
                .collect(),
            use_cases: c
                .use_cases
                .into_iter()
                .map(|a| model::HelpUseCase {
                    id: a.id,
                    title: a.title,
                    description: a.description,
                    example_prompts: a.example_prompts,
                })
                .collect(),
        }
    }
}
#[derive(Serialize)]
pub struct Library {
    id: String,
    name: String,
    version: i64,
    published_revision: i64,
    draft: Content,
    updated_at: String,
}
impl From<model::HelpLibrary> for Library {
    fn from(r: model::HelpLibrary) -> Self {
        Self {
            id: r.id,
            name: r.name,
            version: r.version,
            published_revision: r.published_revision,
            draft: r.draft.into(),
            updated_at: r.updated_at.to_rfc3339(),
        }
    }
}
#[derive(Serialize)]
pub struct Revision {
    revision: i64,
    content: Content,
    published_at: String,
    published_by: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Save {
    version: i64,
    content: Content,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Publish {
    version: i64,
    rollback_revision: Option<i64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryQuery {
    before: Option<i64>,
}
#[derive(Serialize)]
pub struct Assignment {
    profile_id: String,
    library_id: String,
    version: i64,
}
impl From<model::HelpAssignment> for Assignment {
    fn from(a: model::HelpAssignment) -> Self {
        Self {
            profile_id: a.id,
            library_id: a.library_id,
            version: a.version,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assign {
    generation: i64,
    version: i64,
    library_id: uuid::Uuid,
}
fn audit(state: &AppState, auth: &AuthUser, action: &str, id: &str, version: i64) {
    audit_service::log_for_user(
        state.db.clone(),
        auth,
        action,
        Some(serde_json::json!({"library_id":id,"version":version})),
    );
}
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<(HeaderMap, Json<Vec<Library>>)> {
    super::platform_bots::admin(&state, &auth).await?;
    Ok((
        headers(),
        Json(
            service::list(&state.db)
                .await?
                .into_iter()
                .map(Into::into)
                .collect(),
        ),
    ))
}
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<Create>,
) -> AppResult<(HeaderMap, Json<Library>)> {
    super::platform_bots::admin(&state, &auth).await?;
    let r = service::create(&state.db, &body.name, &auth.user_id.to_string()).await?;
    audit(&state, &auth, "platform_help_created", &r.id, r.version);
    Ok((headers(), Json(r.into())))
}
pub async fn save(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<Save>,
) -> AppResult<(HeaderMap, Json<Library>)> {
    super::platform_bots::admin(&state, &auth).await?;
    let r = service::save(
        &state.db,
        &id.to_string(),
        body.version,
        &body.content.into(),
        &auth.user_id.to_string(),
    )
    .await?;
    audit(&state, &auth, "platform_help_draft_saved", &r.id, r.version);
    Ok((headers(), Json(r.into())))
}
pub async fn publish(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<Publish>,
) -> AppResult<(HeaderMap, Json<Library>)> {
    super::platform_bots::admin(&state, &auth).await?;
    let r = service::publish(
        &state.db,
        &id.to_string(),
        body.version,
        &auth.user_id.to_string(),
        body.rollback_revision,
    )
    .await?;
    audit(
        &state,
        &auth,
        if body.rollback_revision.is_some() {
            "platform_help_rolled_back"
        } else {
            "platform_help_published"
        },
        &r.id,
        r.version,
    );
    Ok((headers(), Json(r.into())))
}
pub async fn history(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Query(query): Query<HistoryQuery>,
) -> AppResult<(HeaderMap, Json<Vec<Revision>>)> {
    super::platform_bots::admin(&state, &auth).await?;
    Ok((
        headers(),
        Json(
            service::history(&state.db, &id.to_string(), query.before)
                .await?
                .into_iter()
                .map(|r| Revision {
                    revision: r.revision,
                    content: r.content.into(),
                    published_at: r.published_at.to_rfc3339(),
                    published_by: r.published_by,
                })
                .collect(),
        ),
    ))
}
pub async fn assignment(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
) -> AppResult<(HeaderMap, Json<Assignment>)> {
    super::platform_bots::admin(&state, &auth).await?;
    super::super::services::platform_bot_config_service::get(&state.db, &id.to_string()).await?;
    Ok((
        headers(),
        Json(
            service::assignment(&state.db, &id.to_string())
                .await?
                .into(),
        ),
    ))
}
pub async fn assign(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<Assign>,
) -> AppResult<(HeaderMap, Json<Assignment>)> {
    super::platform_bots::admin(&state, &auth).await?;
    let r = service::assign(
        &state.db,
        &id.to_string(),
        body.generation,
        body.version,
        &body.library_id.to_string(),
        &auth.user_id.to_string(),
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "platform_help_assigned",
        Some(serde_json::json!({"profile_id":id,"library_id":r.library_id,"version":r.version})),
    );
    Ok((headers(), Json(r.into())))
}
