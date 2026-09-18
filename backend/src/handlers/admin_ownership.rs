use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};

use crate::services::{
    audit_service,
    ownership_transfer_service::{self as transfers, ResourceKind, TransferCommand},
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct ResourceQuery {
    pub owner_user_id: Option<String>,
    pub search: Option<String>,
    #[serde(default)]
    pub offset: u64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ResourceItem {
    pub id: String,
    pub name: String,
    pub owner_user_id: String,
    pub platform: Option<String>,
    pub slug: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ResourceListResponse {
    pub items: Vec<ResourceItem>,
    pub next_offset: Option<u64>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewRequest {
    pub new_owner_user_id: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TransferRequest {
    pub new_owner_user_id: String,
    pub request_id: String,
    pub expected_version: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PreviewResponse {
    pub resource_kind: ResourceKind,
    pub resource_id: String,
    pub name: String,
    pub previous_owner_user_id: String,
    pub previous_owner_name: String,
    pub new_owner_user_id: String,
    pub destination_name: String,
    pub destination_type: String,
    pub version: String,
    pub routes_to_retire: u64,
    pub blockers: Vec<String>,
    pub effects: Vec<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TransferResponse {
    pub transfer_id: String,
    pub resource_kind: String,
    pub resource_id: String,
    pub previous_owner_user_id: String,
    pub new_owner_user_id: String,
    pub retired_routes: u64,
}

#[utoipa::path(get, path = "/api/v1/admin/ownership/{kind}",
    params(("kind" = ResourceKind, Path), ResourceQuery),
    responses((status = 200, body = ResourceListResponse), (status = 403, description = "NyxID admin required")),
    tag = "Admin Ownership")]
pub async fn list_resources(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(kind): Path<ResourceKind>,
    Query(query): Query<ResourceQuery>,
) -> AppResult<Json<ResourceListResponse>> {
    transfers::require_platform_admin(&state.db, &auth.user_id.to_string()).await?;
    if query.offset > 100_000 {
        return Err(AppError::ValidationError("Offset is too large".into()));
    }
    let rows = transfers::list_resources(
        &state.db,
        kind,
        query.owner_user_id.as_deref(),
        query.search.as_deref(),
        query.offset,
    )
    .await?;
    let next_offset = (rows.len() > 50).then_some(query.offset + 50);
    let items = rows
        .into_iter()
        .take(50)
        .map(|row| {
            let field = |name| row.get_str(name).ok().map(str::to_owned);
            ResourceItem {
                id: field("_id").unwrap_or_default(),
                name: field("name").or_else(|| field("label")).unwrap_or_default(),
                owner_user_id: field("owner_user_id")
                    .or_else(|| field("user_id"))
                    .or_else(|| field("created_by"))
                    .unwrap_or_default(),
                platform: field("platform"),
                slug: field("slug"),
            }
        })
        .collect();
    Ok(Json(ResourceListResponse { items, next_offset }))
}

#[utoipa::path(post, path = "/api/v1/admin/ownership/{kind}/{id}/preview",
    params(("kind" = ResourceKind, Path), ("id" = String, Path)), request_body = PreviewRequest,
    responses((status = 200, body = PreviewResponse), (status = 403, description = "NyxID admin required")),
    tag = "Admin Ownership")]
pub async fn preview(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((kind, id)): Path<(ResourceKind, String)>,
    Json(body): Json<PreviewRequest>,
) -> AppResult<Json<PreviewResponse>> {
    let preview = transfers::preview(
        &state.db,
        &auth.user_id.to_string(),
        kind,
        &id,
        &body.new_owner_user_id,
        state.config.channel_relay_max_bots_per_user,
    )
    .await?;
    let effects = match kind {
        ResourceKind::Service => vec![
            "Catalog ownership changes; shared service configuration remains managed by NyxID admins.".into(),
            "Existing connections, credentials, visibility, platform-key grants and pricing are preserved.".into(),
        ],
        ResourceKind::ChannelBot => vec![
            "The bot's stored credentials move with it. Provider-side account ownership and existing token copies do not change.".into(),
            "Existing routes are permanently retired. The destination must assign its own agents before messages can be delivered.".into(),
            "Historical conversations and message metadata stay with the previous owner. Already-dispatched external work may finish.".into(),
        ],
    };
    Ok(Json(PreviewResponse {
        resource_kind: preview.kind,
        resource_id: preview.resource_id,
        name: preview.name,
        previous_owner_user_id: preview.previous_owner_user_id,
        previous_owner_name: preview.previous_owner_name,
        new_owner_user_id: preview.new_owner_user_id,
        destination_name: preview.destination_name,
        destination_type: preview.destination_type.into(),
        version: preview.version,
        routes_to_retire: preview.routes_to_retire,
        blockers: preview.blockers,
        effects,
    }))
}

#[utoipa::path(post, path = "/api/v1/admin/ownership/{kind}/{id}/transfer",
    params(("kind" = ResourceKind, Path), ("id" = String, Path)), request_body = TransferRequest,
    responses((status = 200, body = TransferResponse), (status = 403, description = "NyxID admin required"),
        (status = 409, description = "Stale preview or unsupported transfer")),
    tag = "Admin Ownership")]
pub async fn transfer(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((kind, id)): Path<(ResourceKind, String)>,
    Json(body): Json<TransferRequest>,
) -> AppResult<Json<TransferResponse>> {
    let actor = auth.user_id.to_string();
    let receipt = transfers::transfer(
        &state.db,
        TransferCommand {
            actor: &actor,
            kind,
            resource_id: &id,
            destination: &body.new_owner_user_id,
            request_id: &body.request_id,
            expected_version: &body.expected_version,
            capacity: state.config.channel_relay_max_bots_per_user,
        },
    )
    .await?;
    let response = TransferResponse {
        transfer_id: receipt.id,
        resource_kind: receipt.resource_kind,
        resource_id: receipt.resource_id,
        previous_owner_user_id: receipt.previous_owner_user_id,
        new_owner_user_id: receipt.new_owner_user_id,
        retired_routes: receipt.retired_routes,
    };
    audit_service::log_actor_event(
        state.db.clone(),
        &audit_service::AuditActor::from_auth_user(&auth),
        "admin_ownership_transferred",
        Some(serde_json::json!(&response)),
    )
    .await?;
    Ok(Json(response))
}
