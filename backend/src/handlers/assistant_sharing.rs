use axum::{
    Json,
    extract::{Path, State},
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde_json::{Value, json};

use crate::{
    AppState,
    errors::AppResult,
    models::assistant_sharing::{self as model, PrivateNotes},
    mw::auth::AuthUser,
    services::{
        assistant_nyxagent as engine, assistant_team_service as team,
        shared_agent_service as sharing,
    },
};

async fn human(state: &AppState, auth: &AuthUser) -> AppResult<String> {
    super::login_client_context::require_first_party_human(auth)?;
    let actor = auth.user_id.to_string();
    engine::require_enabled(&state.db, &actor).await?;
    Ok(actor)
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let actor = human(&state, &auth).await?;
    let agent = team::agent(&state.db, &actor, &id).await?;
    if agent.user_id != actor {
        let access = sharing::require(&state.db, &actor, &agent).await?;
        return Ok(Json(
            json!({"enabled": true, "role": access.role(), "version": access.version(), "revision": access.revision(), "owner_id": access.owner()}),
        ));
    }
    sharing::personal_owner(&state.db, &actor, &id).await?;
    let policy = sharing::policy(&state.db, &agent).await?;
    let audience = sharing::audience_for_owner(&state.db, &actor, &id).await?;
    let services: Vec<crate::models::user_service::UserService> = state
        .db
        .collection(crate::models::user_service::COLLECTION_NAME)
        .find(
            doc! {"user_id": &actor, "is_active": true, "$or": [
                {"_id": {"$in": &agent.grants.service_ids}},
                {"catalog_service_id": {"$in": &agent.grants.platform_service_ids}, "credential_binding": "platform"}]},
        )
        .await?
        .try_collect()
        .await?;
    let catalog_ids: Vec<_> = services
        .iter()
        .filter_map(|s| s.catalog_service_id.as_ref())
        .collect();
    let catalogs: Vec<mongodb::bson::Document> = state.db.collection("downstream_services")
        .find(doc! {"_id": {"$in": catalog_ids}, "is_active": true, "service_category": {"$ne": "internal"}})
        .projection(doc! {"_id": 1}).await?.try_collect().await?;
    let mut services: Vec<_> = services
        .into_iter()
        .filter(sharing::server_service)
        .filter(|s| {
            s.catalog_service_id
                .as_ref()
                .is_none_or(|id| catalogs.iter().any(|c| c.get_str("_id") == Ok(id.as_str())))
        })
        .filter_map(|s| {
            sharing::owner_grant(&agent, &s).map(|id| json!({"id": id, "name": s.slug}))
        })
        .collect();
    services.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    services.dedup_by(|a, b| a["id"] == b["id"]);
    Ok(Json(
        json!({"enabled": policy.enabled, "role": "owner", "version": policy.version, "revision": policy.revision,
        "owner_id": actor, "audience": audience, "shared_access": policy.shared_access, "services": services}),
    ))
}

pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<sharing::SharingUpdate>,
) -> AppResult<Json<Value>> {
    let actor = human(&state, &auth).await?;
    let row = sharing::update(&state.db, &actor, &id, body, true).await?;
    Ok(Json(
        json!({"enabled": row.enabled, "version": row.version, "revision": row.revision}),
    ))
}

pub async fn private_notes(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let actor = human(&state, &auth).await?;
    // Actor-only history remains readable after unshare; never enumerate another person's notes.
    let row = state
        .db
        .collection::<PrivateNotes>(model::NOTES_COLLECTION_NAME)
        .find_one(doc! {"agent_id": &id, "actor_user_id": &actor})
        .await?;
    let notes: Vec<_> = row
        .as_ref()
        .map(|r| {
            r.notes
                .iter()
                .map(|n| json!({"id": n.id, "text": n.text}))
                .collect()
        })
        .unwrap_or_default();
    Ok(Json(
        json!({"revision": row.map_or(0, |r| r.revision), "notes": notes}),
    ))
}

pub async fn update_notes(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<sharing::NotesUpdate>,
) -> AppResult<Json<Value>> {
    let actor = human(&state, &auth).await?;
    let agent = team::agent(&state.db, &actor, &id).await?;
    let row = sharing::update_notes(&state.db, &actor, &agent, body).await?;
    Ok(Json(json!({"revision": row.revision})))
}

pub async fn delete_notes(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let actor = human(&state, &auth).await?;
    state
        .db
        .collection::<PrivateNotes>(model::NOTES_COLLECTION_NAME)
        .delete_one(doc! {"agent_id": id, "actor_user_id": actor})
        .await?;
    Ok(Json(json!({"deleted": true})))
}

pub async fn memory(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let actor = human(&state, &auth).await?;
    let agent = team::agent(&state.db, &actor, &id).await?;
    if actor == agent.user_id {
        sharing::personal_owner(&state.db, &actor, &id).await?;
    } else {
        sharing::require(&state.db, &actor, &agent).await?;
    }
    let row = sharing::policy(&state.db, &agent).await?;
    let notes: Vec<_> = row
        .memory
        .iter()
        .map(|n| json!({"id": n.id, "text": n.text}))
        .collect();
    Ok(Json(json!({"revision": row.revision, "notes": notes})))
}

pub async fn update_memory(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<sharing::NotesUpdate>,
) -> AppResult<Json<Value>> {
    let actor = human(&state, &auth).await?;
    let agent = team::agent(&state.db, &actor, &id).await?;
    let row = sharing::update_memory(&state.db, &actor, &agent, body).await?;
    Ok(Json(json!({"revision": row.revision})))
}

pub async fn consents(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let actor = human(&state, &auth).await?;
    let rows: Vec<model::AgentConsent> = state.db.collection(model::CONSENTS_COLLECTION_NAME)
        .find(doc! {"actor_user_id": &actor, "agent_id": id, "decision": {"$in": ["pending", "always", "once"]},
            "$or": [{"expires_at": null}, {"expires_at": {"$gt": mongodb::bson::DateTime::now()}}]})
        .sort(doc! {"created_at": -1}).limit(100).await?.try_collect().await?;
    let connections: Vec<crate::models::user_service::UserService> = state.db.collection("user_services")
        .find(doc! {"user_id": &actor, "_id": {"$in": rows.iter().map(|r| &r.connection_id).collect::<Vec<_>>()}})
        .await?.try_collect().await?;
    let rows: Vec<_> = rows
        .into_iter()
        .map(|r| {
            let connection_name = connections.iter().find(|c| c.id == r.connection_id).map(|c| c.slug.as_str());
            json!({"id": r.id, "connection_id": r.connection_id, "connection_name": connection_name,
        "catalog_service_id": r.catalog_service_id, "conversation_id": r.conversation_id,
        "operation": r.operation_summary, "decision": r.decision, "expires_at": r.expires_at, "scope": if r.stateless_model { "stateless_model_turn" } else { "exact_operation" }})
        })
        .collect();
    Ok(Json(json!({"consents": rows})))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentDecision {
    pub decision: model::ConsentDecision,
}

pub async fn decide_consent(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<ConsentDecision>,
) -> AppResult<Json<Value>> {
    let actor = human(&state, &auth).await?;
    crate::services::shared_agent_consent::decide(&state.db, &actor, &id, body.decision).await?;
    Ok(Json(json!({"decided": true})))
}
