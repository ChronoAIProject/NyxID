use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};

use crate::services::{
    audit_service, ownership_transfer_access as access,
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
    tag = "Ownership")]
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

#[utoipa::path(post, path = "/api/v1/ownership/{kind}/{id}/preview",
    params(("kind" = ResourceKind, Path), ("id" = String, Path)), request_body = PreviewRequest,
    responses((status = 200, body = PreviewResponse), (status = 403, description = "Asset owner, organization admin, or platform admin required")),
    tag = "Ownership")]
pub async fn preview(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((kind, id)): Path<(ResourceKind, String)>,
    Json(body): Json<PreviewRequest>,
) -> AppResult<Json<PreviewResponse>> {
    auth.ensure_write_scope()?;
    let preview = transfers::preview_with_agent(
        &state.db,
        &auth.user_id.to_string(),
        auth.api_key_id.as_deref(),
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
        ResourceKind::ChannelBot => {
            let mut effects = Vec::new();
            if preview.moves_oauth_credential {
                effects.push("The bot's dedicated X connection moves to the new owner. The connected X account and existing token copies are unchanged.".into());
            } else {
                effects.push("The bot's stored credentials move with it. Provider-side account ownership and existing token copies do not change.".into());
            }
            effects.extend([
                "Existing routes are permanently retired. The destination must assign its own agents before messages can be delivered.".into(),
                "Historical conversations and message metadata stay with the previous owner. Already-dispatched external work may finish.".into(),
            ]);
            effects
        }
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

#[utoipa::path(post, path = "/api/v1/ownership/{kind}/{id}/transfer",
    params(("kind" = ResourceKind, Path), ("id" = String, Path)), request_body = TransferRequest,
    responses((status = 200, body = TransferResponse), (status = 403, description = "Asset owner, organization admin, or platform admin required"),
        (status = 409, description = "Stale preview or unsupported transfer")),
    tag = "Ownership")]
pub async fn transfer(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((kind, id)): Path<(ResourceKind, String)>,
    Json(body): Json<TransferRequest>,
) -> AppResult<Json<TransferResponse>> {
    auth.ensure_write_scope()?;
    let actor = auth.user_id.to_string();
    let receipt = transfers::transfer(
        &state.db,
        TransferCommand {
            actor: &actor,
            api_key_id: auth.api_key_id.as_deref(),
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

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TransferAuthorization {
    pub can_transfer: bool,
    pub resource: Option<ResourceItem>,
}

#[utoipa::path(get, path = "/api/v1/ownership/{kind}/{id}/authorization",
    params(("kind" = ResourceKind, Path), ("id" = String, Path)),
    responses((status = 200, body = TransferAuthorization)), tag = "Ownership")]
pub async fn authorization(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((kind, id)): Path<(ResourceKind, String)>,
) -> AppResult<Json<TransferAuthorization>> {
    if matches!(auth.auth_method, crate::mw::auth::AuthMethod::Delegated) {
        return Ok(Json(TransferAuthorization {
            can_transfer: false,
            resource: None,
        }));
    }
    let result = access::require_resource_access(
        &state.db,
        &auth.user_id.to_string(),
        auth.api_key_id.as_deref(),
        kind,
        &id,
    )
    .await;
    let resource = match result {
        Ok((owner, _, row)) if auth.can_write() => Some(ResourceItem {
            id,
            name: row
                .get_str("name")
                .or_else(|_| row.get_str("label"))
                .unwrap_or_default()
                .into(),
            owner_user_id: owner,
            platform: row.get_str("platform").ok().map(str::to_owned),
            slug: row.get_str("slug").ok().map(str::to_owned),
        }),
        Ok(_) | Err(AppError::Forbidden(_) | AppError::NotFound(_)) => None,
        Err(error) => return Err(error),
    };
    Ok(Json(TransferAuthorization {
        can_transfer: resource.is_some(),
        resource,
    }))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct DestinationQuery {
    pub user_type: String,
    #[serde(default)]
    pub search: String,
    #[serde(default)]
    pub offset: u64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DestinationItem {
    pub id: String,
    pub display_name: Option<String>,
    pub email: String,
    pub is_active: bool,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DestinationList {
    pub users: Vec<DestinationItem>,
    pub total: u64,
}

#[utoipa::path(get, path = "/api/v1/ownership/{kind}/{id}/destinations",
    params(("kind" = ResourceKind, Path), ("id" = String, Path), DestinationQuery),
    responses((status = 200, body = DestinationList), (status = 403, description = "Transfer authority required")), tag = "Ownership")]
pub async fn destinations(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((kind, id)): Path<(ResourceKind, String)>,
    Query(query): Query<DestinationQuery>,
) -> AppResult<Json<DestinationList>> {
    auth.ensure_write_scope()?;
    let actor = auth.user_id.to_string();
    let (owner, admin, _) =
        access::require_resource_access(&state.db, &actor, auth.api_key_id.as_deref(), kind, &id)
            .await?;
    let (rows, total) = access::destination_candidates(
        &state.db,
        &actor,
        &owner,
        admin,
        &query.user_type,
        &query.search,
        query.offset,
    )
    .await?;
    let users = rows
        .into_iter()
        .map(|row| DestinationItem {
            id: row.get_str("_id").unwrap_or_default().into(),
            display_name: row.get_str("display_name").ok().map(str::to_owned),
            email: row.get_str("email").unwrap_or_default().into(),
            is_active: true,
        })
        .collect();
    Ok(Json(DestinationList { users, total }))
}
