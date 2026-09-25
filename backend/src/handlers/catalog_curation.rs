use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    middleware,
    routing::{get, post},
};
use futures::TryStreamExt;
use mongodb::bson::{Document, doc};
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        catalog_skill_revision::{
            COLLECTION_NAME as HISTORY, CatalogSkillRevision, SkillReference, SkillState,
        },
        downstream_service::{
            COLLECTION_NAME as SERVICES, DownstreamService, legacy_http_service_type_filter,
        },
        service_account::{ServiceAccount, ServiceAccountPurpose},
    },
    mw::auth::{
        AuthMethod, AuthUser, reject_api_key_tokens, reject_delegated_tokens, reject_relay_tokens,
    },
    services::{
        api_docs_service, audit_service, catalog_editor_service as editor,
        catalog_skill_service::{self as skills, SkillActor, SkillUpdate},
        catalog_spec_registry, curation_grant_service as grants, service_account_service,
    },
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/services", get(list_services))
        .route("/services/{id}/skills", get(get_skills).put(update_skills))
        .route("/services/{id}/openapi.json", get(get_openapi))
        .route("/services/{id}/skills/history", get(history))
        .route("/services/{id}/skills/restore", post(restore))
        .layer(DefaultBodyLimit::max(70_000))
        .layer(middleware::from_fn(reject_api_key_tokens))
        .layer(middleware::from_fn(reject_delegated_tokens))
        .layer(middleware::from_fn(reject_relay_tokens))
}

async fn authorize(
    state: &AppState,
    auth: &AuthUser,
    scope: &str,
    service: Option<&str>,
) -> AppResult<ServiceAccount> {
    if auth.auth_method != AuthMethod::ServiceAccount {
        return Err(AppError::Forbidden(
            "Curation requires a service-account token".into(),
        ));
    }
    let sa =
        service_account_service::get_service_account(&state.db, &auth.user_id.to_string()).await?;
    if sa.purpose == ServiceAccountPurpose::CatalogEditor {
        editor::authorize(&state.db, &sa, &auth.scope, scope).await?;
    } else {
        grants::live_grant(&sa)?;
        grants::require_scope(&sa, &auth.scope, scope)?;
        if let Some(id) = service {
            grants::require_service(&sa, &auth.scope, scope, id)?;
        }
    }
    Ok(sa)
}

#[derive(Serialize)]
pub struct SkillsResponse {
    pub service_id: String,
    pub recommended_skills: Option<Vec<String>>,
    pub recommended_skill_refs: Option<Vec<SkillReference>>,
    pub skills_revision: i64,
    pub skills_manifest_digest: String,
}

impl SkillsResponse {
    fn new(service_id: String, revision: i64, state: SkillState) -> Self {
        Self {
            service_id,
            skills_revision: revision,
            skills_manifest_digest: skills::manifest_digest(&state),
            recommended_skills: state.recommended_skills,
            recommended_skill_refs: state.recommended_skill_refs,
        }
    }
}

#[derive(Serialize)]
struct ServiceSummary {
    id: String,
    slug: String,
    name: String,
    skills_revision: i64,
}
#[derive(Serialize)]
struct ServicesResponse {
    services: Vec<ServiceSummary>,
}

async fn list_services(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<ServicesResponse>> {
    let sa = authorize(&state, &auth, grants::READ_SCOPE, None).await?;
    let filter = if sa.purpose == ServiceAccountPurpose::CatalogEditor {
        doc! {}
    } else {
        doc! {"_id": {"$in": &grants::live_grant(&sa)?.service_ids}}
    };
    let rows: Vec<DownstreamService> = state
        .db
        .collection::<DownstreamService>(SERVICES)
        .find(filter)
        .sort(doc! {"_id": 1})
        .await?
        .try_collect()
        .await?;
    Ok(Json(ServicesResponse {
        services: rows
            .into_iter()
            .map(|s| ServiceSummary {
                id: s.id,
                slug: s.slug,
                name: s.name,
                skills_revision: s.skills_revision,
            })
            .collect(),
    }))
}

/// Return the operation contract for one grant-listed catalog service.
///
/// This is deliberately a Curation route rather than a general proxy/docs
/// route. It accepts only a catalog service ID present in the live grant and
/// performs a bounded, read-only OpenAPI fetch. It never resolves a
/// user-managed service, injects a credential, or exposes `/mcp/config`'s
/// caller-wide operation catalog.
async fn get_openapi(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    authorize(&state, &auth, grants::READ_SCOPE, Some(&id)).await?;

    let mut service_filter = doc! {"_id": &id, "is_active": true};
    service_filter.extend(legacy_http_service_type_filter());
    let service = state
        .db
        .collection::<DownstreamService>(SERVICES)
        .find_one(service_filter)
        .await?
        .ok_or_else(|| AppError::NotFound("Service not found".into()))?;

    // Seeded catalog services may use an embedded overlay. Serving that value
    // directly avoids a loop back through this deployment when the stored URL
    // points at `/api/v1/catalog-specs/...` and keeps the contract available in
    // test/air-gapped environments. An explicit downstream URL wins over
    // the Curation route's slug fallback. Return the source contract for
    // authoring; this does not grant execution or rewrite proxy routing.
    let configured_url = service
        .openapi_spec_url
        .as_deref()
        .filter(|url| !url.trim().is_empty());
    let spec = if let Some(url) = configured_url {
        let spec = api_docs_service::fetch_spec_json(url).await?;
        if spec.get("openapi").is_none() && spec.get("swagger").is_none() {
            return Err(AppError::BadRequest(
                "Downstream spec is not an OpenAPI or Swagger document".into(),
            ));
        }
        spec.as_ref().clone()
    } else {
        catalog_spec_registry::spec_for_slug(&service.slug)
            .ok_or_else(|| AppError::NotFound("Service has no OpenAPI spec configured".into()))?
            .as_ref()
            .clone()
    };

    Ok(Json(spec))
}

async fn get_skills(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<SkillsResponse>> {
    authorize(&state, &auth, grants::READ_SCOPE, Some(&id)).await?;
    let service = state
        .db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"_id": &id})
        .await?
        .ok_or_else(|| AppError::NotFound("Service not found".into()))?;
    Ok(Json(SkillsResponse::new(
        id,
        service.skills_revision,
        skills::state(&service),
    )))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateSkillsRequest {
    recommended_skills: Option<Vec<String>>,
    recommended_skill_refs: Option<Vec<SkillReference>>,
    #[serde(default)]
    clear_refs: bool,
    base_revision: i64,
    request_id: String,
}

fn actor(auth: &AuthUser) -> AppResult<SkillActor> {
    Ok(SkillActor::Curation {
        id: auth.user_id.to_string(),
        scope: auth.scope.clone(),
        token_jti: auth
            .token_jti
            .clone()
            .ok_or_else(|| AppError::Unauthorized("Service account token required".into()))?,
    })
}

async fn update_skills(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateSkillsRequest>,
) -> AppResult<Json<SkillsResponse>> {
    authorize(&state, &auth, grants::WRITE_SCOPE, Some(&id)).await?;
    let input = SkillUpdate {
        recommended_skills: body.recommended_skills,
        recommended_skill_refs: body.recommended_skill_refs,
        clear_refs: body.clear_refs,
    };
    if !input.is_present() {
        return Err(AppError::ValidationError(
            "A complete recommendation list or clear_refs is required".into(),
        ));
    }
    let committed = skills::commit(
        &state.db,
        &id,
        &actor(&auth)?,
        &input,
        body.base_revision,
        &body.request_id,
        &Document::new(),
        &Document::new(),
        None,
        None,
    )
    .await?;
    if committed.changed {
        audit_change(&state, &auth, &id, committed.revision, &body.request_id);
    }
    Ok(Json(SkillsResponse::new(
        id,
        committed.revision,
        committed.state,
    )))
}

fn audit_change(state: &AppState, auth: &AuthUser, id: &str, revision: i64, request_id: &str) {
    audit_service::log_for_user(
        state.db.clone(),
        auth,
        "catalog_skills_updated",
        Some(
            serde_json::json!({"service_id": id, "skills_revision": revision, "request_id": request_id}),
        ),
    );
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RestoreRequest {
    revision: i64,
    base_revision: i64,
    request_id: String,
}

async fn restore(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<RestoreRequest>,
) -> AppResult<Json<SkillsResponse>> {
    authorize(&state, &auth, grants::WRITE_SCOPE, Some(&id)).await?;
    let committed = skills::commit(
        &state.db,
        &id,
        &actor(&auth)?,
        &SkillUpdate::default(),
        body.base_revision,
        &body.request_id,
        &Document::new(),
        &Document::new(),
        Some(body.revision),
        None,
    )
    .await?;
    if committed.changed {
        audit_change(&state, &auth, &id, committed.revision, &body.request_id);
    }
    Ok(Json(SkillsResponse::new(
        id,
        committed.revision,
        committed.state,
    )))
}

#[derive(Deserialize)]
struct HistoryQuery {
    before_revision: Option<i64>,
    limit: Option<i64>,
}

#[derive(Serialize)]
struct HistoryItem {
    revision: i64,
    previous_revision: i64,
    previous: SkillsResponse,
    current: SkillsResponse,
    actor_kind: String,
    actor_id: String,
    grant_id: Option<String>,
    request_id: String,
    created_at: String,
}
#[derive(Serialize)]
struct HistoryResponse {
    history: Vec<HistoryItem>,
    next_before_revision: Option<i64>,
}

async fn history(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> AppResult<Json<HistoryResponse>> {
    authorize(&state, &auth, grants::READ_SCOPE, Some(&id)).await?;
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let mut filter = doc! {"service_id": &id};
    if let Some(before) = query.before_revision {
        filter.insert("revision", doc! {"$lt": before});
    }
    let mut rows: Vec<CatalogSkillRevision> = state
        .db
        .collection::<CatalogSkillRevision>(HISTORY)
        .find(filter)
        .sort(doc! {"revision": -1})
        .limit(limit + 1)
        .await?
        .try_collect()
        .await?;
    let next_before_revision = if rows.len() as i64 > limit {
        rows.truncate(limit as usize);
        rows.last().map(|r| r.revision)
    } else {
        None
    };
    Ok(Json(HistoryResponse {
        next_before_revision,
        history: rows
            .into_iter()
            .map(|r| HistoryItem {
                revision: r.revision,
                previous_revision: r.previous_revision,
                previous: SkillsResponse::new(id.clone(), r.previous_revision, r.previous),
                current: SkillsResponse::new(id.clone(), r.revision, r.current),
                actor_kind: r.actor_kind,
                actor_id: r.actor_id,
                grant_id: r.grant_id,
                request_id: r.request_id,
                created_at: r.created_at.to_rfc3339(),
            })
            .collect(),
    }))
}
