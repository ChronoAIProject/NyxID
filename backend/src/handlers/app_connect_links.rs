//! Repair creation/read accepts developer-app user tokens. Hosted actions require
//! the bound human's Session authentication; org resource administration never
//! grants access to another subject's session. All reads are local observations.

use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::HeaderMap,
};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use super::app_requirements::require_app_user;
use crate::models::app_connect_link::{
    AppConnectLink, AppConnectOrigin, AppConnectStatus, ItemState,
};
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::{
    app_connect_link_service as links, audit_service, connect_link_service, validator_profiles,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    pub callback_url: String,
    pub state: String,
}
#[derive(Serialize)]
pub struct CreateResponse {
    pub id: String,
    pub connect_url: String,
    pub expires_at: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedeemRequest {
    pub capability: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectRequest {
    pub service_slug: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectRequest {
    pub user_service_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChoiceResponse {
    pub user_service_id: String,
    pub slug: String,
    pub owner_id: String,
}
#[derive(Debug, Serialize)]
pub struct ItemResponse {
    pub requirement_id: String,
    pub label: String,
    pub optional: bool,
    pub state: ItemState,
    pub readiness: &'static str,
    pub user_service_id: Option<String>,
    pub slug: Option<String>,
    pub resource_uri: Option<String>,
    pub owner_id: Option<String>,
    pub connect_link_id: Option<String>,
    pub reason_code: Option<String>,
    pub claim: Option<&'static str>,
    pub validated_at: Option<String>,
    pub valid_until: Option<String>,
    pub granted_to_caller: bool,
    pub catalog_slugs: Vec<String>,
    pub required_scopes: Vec<String>,
    pub choices: Vec<ChoiceResponse>,
}
#[derive(Serialize)]
pub struct LinkResponse {
    pub id: String,
    pub oauth_client_id: String,
    pub client_name: String,
    pub handoff_blurb: Option<String>,
    pub destination: String,
    pub requirements_version: u32,
    pub status: AppConnectStatus,
    pub expires_at: String,
    pub items: Vec<ItemResponse>,
    pub callback_url: Option<String>,
    pub grant_update_required: bool,
}
#[derive(Serialize)]
pub struct ChildResponse {
    pub id: String,
    pub token: String,
    pub service_name: String,
    pub service_slug: String,
    pub connect_method: String,
    pub auth_key_name: String,
    pub credential_mode: Option<String>,
    pub has_platform_oauth_credentials: bool,
    pub requires_gateway_url: bool,
    pub api_key_url: Option<String>,
    pub api_key_instructions: Option<String>,
}

pub async fn require_human(state: &AppState, auth: &AuthUser) -> AppResult<String> {
    if auth.auth_method != AuthMethod::Session
        || auth.acting_client_id.is_some()
        || auth.api_key_id.is_some()
    {
        return Err(AppError::Forbidden("A human session is required".into()));
    }
    let subject = auth.user_id.to_string();
    let user = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &subject, "is_active": true })
        .await?;
    if !user.is_some_and(|u| u.user_type == UserType::Person) {
        return Err(AppError::Forbidden("A human session is required".into()));
    }
    Ok(subject)
}

async fn limit_ip(state: &AppState, headers: &HeaderMap, addr: SocketAddr) -> AppResult<()> {
    let ip = crate::mw::rate_limit::resolve_client_ip_for_rate_limit(
        headers,
        Some(addr),
        &state.config.trusted_proxy_ips,
    )
    .unwrap_or(addr.ip());
    if !state.connect_link_complete_limiter.check_shared(ip).await? {
        return Err(AppError::RateLimited);
    }
    Ok(())
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateRequest>,
) -> AppResult<Json<CreateResponse>> {
    let client_id = require_app_user(&state, &auth).await?;
    links::enabled_client(&state, &client_id).await?;
    let subject = auth.user_id.to_string();
    if !state
        .connect_link_create_limiter
        .check_shared(&subject)
        .await?
    {
        return Err(AppError::RateLimited);
    }
    let created = links::start_from_app(
        &state,
        &client_id,
        &subject,
        &body.callback_url,
        &body.state,
    )
    .await?;
    audit(
        &state,
        &auth,
        "app_connect_link_created",
        &created.link,
        None,
    );
    Ok(Json(CreateResponse {
        id: created.link.id,
        connect_url: created.connect_url,
        expires_at: created.link.expires_at.to_rfc3339(),
    }))
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> AppResult<Json<LinkResponse>> {
    let subject = auth.user_id.to_string();
    let link = if auth.auth_method == AuthMethod::Session {
        require_human(&state, &auth).await?;
        let link = links::load(&state, &id, &subject).await?;
        links::ensure_redeemed(&link)?;
        limit_ip(&state, &headers, addr).await?;
        link
    } else {
        let client = require_app_user(&state, &auth).await?;
        links::enabled_client(&state, &client).await?;
        let link = links::load(&state, &id, &subject).await?;
        if link.oauth_client_id != client {
            return Err(AppError::AppConnectLinkNotFound);
        }
        if !state
            .app_requirements_status_limiter
            .check_shared(&subject)
            .await?
        {
            return Err(AppError::RateLimited);
        }
        link
    };
    response(&state, &auth, &link.id).await.map(Json)
}

pub async fn redeem(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<RedeemRequest>,
) -> AppResult<Json<LinkResponse>> {
    let subject = require_human(&state, &auth).await?;
    links::load(&state, &id, &subject).await?;
    limit_ip(&state, &headers, addr).await?;
    let link = links::redeem(&state, &id, &subject, &body.capability).await?;
    audit(&state, &auth, "app_connect_link_redeemed", &link, None);
    response(&state, &auth, &id).await.map(Json)
}

async fn hosted(
    state: &AppState,
    auth: &AuthUser,
    id: &str,
    headers: &HeaderMap,
    addr: SocketAddr,
) -> AppResult<String> {
    let subject = require_human(state, auth).await?;
    let link = links::load(state, id, &subject).await?;
    links::ensure_redeemed(&link)?;
    links::ensure_open(&link)?;
    limit_ip(state, headers, addr).await?;
    Ok(subject)
}

pub async fn connect(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, requirement)): Path<(String, String)>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<ConnectRequest>,
) -> AppResult<Json<ChildResponse>> {
    let subject = hosted(&state, &auth, &id, &headers, addr).await?;
    child(
        &state,
        &auth,
        &id,
        &subject,
        &requirement,
        &body.service_slug,
        false,
    )
    .await
    .map(Json)
}

pub async fn reauthorize(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, requirement)): Path<(String, String)>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<ConnectRequest>,
) -> AppResult<Json<ChildResponse>> {
    let subject = hosted(&state, &auth, &id, &headers, addr).await?;
    child(
        &state,
        &auth,
        &id,
        &subject,
        &requirement,
        &body.service_slug,
        true,
    )
    .await
    .map(Json)
}

async fn child(
    state: &AppState,
    auth: &AuthUser,
    id: &str,
    subject: &str,
    requirement: &str,
    slug: &str,
    reauthorize: bool,
) -> AppResult<ChildResponse> {
    let created = links::connect_item(state, id, subject, requirement, slug, reauthorize).await?;
    let catalog =
        connect_link_service::load_catalog_info_by_id(&state.db, &created.link.service_id).await?;
    let link = links::load(state, id, subject).await?;
    audit(
        state,
        auth,
        if reauthorize {
            "app_connect_item_reauthorizing"
        } else {
            "app_connect_item_connecting"
        },
        &link,
        Some(requirement),
    );
    let method = catalog.connect_method().into();
    Ok(ChildResponse {
        id: created.link.id,
        token: created.raw_token,
        service_name: catalog.service_name,
        service_slug: catalog.service_slug,
        connect_method: method,
        auth_key_name: catalog.auth_key_name,
        credential_mode: catalog.credential_mode,
        has_platform_oauth_credentials: catalog.has_platform_oauth_credentials,
        requires_gateway_url: catalog.requires_gateway_url,
        api_key_url: catalog.api_key_url,
        api_key_instructions: catalog.api_key_instructions,
    })
}

pub async fn select(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, requirement)): Path<(String, String)>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<SelectRequest>,
) -> AppResult<Json<LinkResponse>> {
    let subject = hosted(&state, &auth, &id, &headers, addr).await?;
    links::select_item(
        &state,
        &id,
        &subject,
        &requirement,
        body.user_service_id.as_deref(),
    )
    .await?;
    response(&state, &auth, &id).await.map(Json)
}

pub async fn validate(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, requirement)): Path<(String, String)>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> AppResult<Json<LinkResponse>> {
    let subject = hosted(&state, &auth, &id, &headers, addr).await?;
    if !state
        .service_validation_limiter
        .check_shared(&subject)
        .await?
    {
        return Err(AppError::ServiceValidationRateLimited);
    }
    links::validate_item(&state, &id, &subject, &requirement).await?;
    response(&state, &auth, &id).await.map(Json)
}

pub async fn ready(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> AppResult<Json<LinkResponse>> {
    let subject = hosted(&state, &auth, &id, &headers, addr).await?;
    let link = links::ready(&state, &id, &subject).await?;
    audit(&state, &auth, "app_connect_link_completed", &link, None);
    response(&state, &auth, &id).await.map(Json)
}

pub async fn cancel(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> AppResult<Json<LinkResponse>> {
    let subject = require_human(&state, &auth).await?;
    let link = links::load(&state, &id, &subject).await?;
    links::ensure_redeemed(&link)?;
    limit_ip(&state, &headers, addr).await?;
    let link = links::cancel(&state, &id, &subject).await?;
    audit(&state, &auth, "app_connect_link_cancelled", &link, None);
    response(&state, &auth, &id).await.map(Json)
}

async fn response(state: &AppState, auth: &AuthUser, id: &str) -> AppResult<LinkResponse> {
    let (link, report) = links::refresh(state, id, &auth.user_id.to_string()).await?;
    let client = links::enabled_client(state, &link.oauth_client_id).await?;
    let manifest = links::manifest(state, &link).await?;
    let mut items = Vec::new();
    for r in report.requirements {
        let required = links::requirement(&manifest, &r.requirement_id)?;
        let item = link
            .items
            .iter()
            .find(|i| i.requirement_id == r.requirement_id)
            .ok_or(AppError::AppConnectResultMismatch)?;
        let choices = if auth.auth_method == AuthMethod::Session {
            links::choices(state, &link, &manifest, required)
                .await?
                .into_iter()
                .map(|s| ChoiceResponse {
                    user_service_id: s.id,
                    slug: s.slug,
                    owner_id: s.user_id,
                })
                .collect()
        } else {
            vec![]
        };
        let claim = match &required.validator {
            crate::models::app_requirement_manifest::ValidatorSelection::Profile { id } => {
                validator_profiles::PROFILES
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.claim)
            }
            _ => None,
        };
        let granted = auth.allow_all_services
            || r.user_service_id
                .as_ref()
                .is_some_and(|id| auth.allowed_service_ids.contains(id));
        items.push(ItemResponse {
            requirement_id: r.requirement_id,
            label: required.label.clone(),
            optional: required.optional,
            state: item.state,
            readiness: r.state.as_str(),
            user_service_id: r.user_service_id,
            slug: r.slug,
            owner_id: r.owner_id,
            resource_uri: r.resource_uri,
            connect_link_id: item.connect_link_id.clone(),
            reason_code: item.reason_code.clone(),
            claim,
            validated_at: r.validated_at.map(|t| t.to_rfc3339()),
            valid_until: r.valid_until.map(|t| t.to_rfc3339()),
            granted_to_caller: granted,
            catalog_slugs: required.any_of_catalog_slugs.clone(),
            required_scopes: required.required_downstream_scopes.clone(),
            choices,
        });
    }
    let destination = match &link.origin {
        AppConnectOrigin::App { callback_url, .. } => {
            let url =
                url::Url::parse(callback_url).map_err(|_| AppError::AppConnectResultMismatch)?;
            if matches!(url.scheme(), "http" | "https") {
                url.host_str().unwrap_or_default().to_string()
            } else {
                format!("desktop app registered as {}", client.client_name)
            }
        }
        _ => return Err(AppError::AppConnectLinkNotFound),
    };
    Ok(LinkResponse {
        id: link.id.clone(),
        oauth_client_id: link.oauth_client_id.clone(),
        client_name: client.client_name,
        handoff_blurb: client.handoff_blurb,
        destination,
        requirements_version: link.manifest_version,
        status: link.status,
        expires_at: link.expires_at.to_rfc3339(),
        items,
        callback_url: links::terminal_callback_url(&link)?,
        grant_update_required: link.grant_update_required,
    })
}

fn audit(
    state: &AppState,
    auth: &AuthUser,
    action: &str,
    link: &AppConnectLink,
    requirement: Option<&str>,
) {
    audit_service::log_for_user(
        state.db.clone(),
        auth,
        action,
        Some(serde_json::json!({
            "app_connect_link_id": link.id, "client_id": link.oauth_client_id, "manifest_id": link.manifest_id,
            "requirement_id": requirement, "status": link.status,
        })),
    );
}

/// Enforce the parent session boundary on the existing single-service endpoints.
pub(crate) async fn guard_child(
    state: &AppState,
    auth: &AuthUser,
    child: &crate::models::connect_link::ConnectLink,
) -> AppResult<()> {
    if let Some(id) = &child.parent_session_id {
        let subject = require_human(state, auth).await?;
        let parent = links::load(state, id, &subject).await?;
        links::ensure_redeemed(&parent)?;
        links::ensure_open(&parent)?;
        links::ensure_child_subject(&state.db, child, &subject).await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "app_connect_links_tests.rs"]
mod tests;
