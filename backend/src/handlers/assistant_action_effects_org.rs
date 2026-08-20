//! Wave-4 organization and account-family assistant action effects.
//!
//! The browser is the only place that collects confirmation values and
//! one-time material. Every committing path reserves a durable receipt before
//! the mutation, fingerprints all non-secret semantic content, and treats a
//! pending receipt as a distinct state that may only be resumed after proving
//! the requested state already exists.

#![allow(dead_code)]

use std::sync::LazyLock;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::post,
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::{approvals, consent, mfa, users};
use crate::models::approval_grant::{ApprovalGrant, COLLECTION_NAME as APPROVAL_GRANTS};
use crate::models::consent::{COLLECTION_NAME as CONSENTS, Consent};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::mfa_factor::{COLLECTION_NAME as MFA_FACTORS, MfaFactor};
use crate::models::service_approval_config::{
    ApprovalEffect, ApprovalMode, ApprovalRule, COLLECTION_NAME as SERVICE_APPROVAL_CONFIGS,
    ServiceApprovalConfig,
};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::mw::auth::AuthUser;
use crate::services::assistant_action_receipts::{
    ReceiptOutcome, fingerprint_canonical, in_progress_conflict, mark_completed,
    normalize_action_request_id, reserve_or_replay,
};
use crate::services::{approval_service, org_service};
use crate::telemetry::TelemetryContext;

const ACCOUNT_PROFILE_UPDATE_ACTION: &str = "account.profile_update";
const ACCOUNT_REVOKE_CONSENT_ACTION: &str = "account.revoke_consent";
const ACCOUNT_DELETE_ACTION: &str = "account.delete";
const ACCOUNT_MFA_SETUP_ACTION: &str = "account.mfa_setup";
const ACCOUNT_MFA_SETUP_START_ACTION: &str = "account.mfa_setup.start";
const APPROVAL_CONFIGURE_ACTION: &str = "approval.configure";
const APPROVAL_ENABLE_ACTION: &str = "approval.enable";
const APPROVAL_DISABLE_ACTION: &str = "approval.disable";
const APPROVAL_REVOKE_GRANT_ACTION: &str = "approval.revoke_grant";

static SECRET_SHAPED_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:Bearer\s+\S+|nyxid_(?:ag_)?[A-Za-z0-9_-]{16,})")
        .expect("secret-shape regex")
});

/// Effect routes mounted at `/api/v1/assistant/actions/org`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/account/profile-update", post(update_account_profile))
        .route("/account/revoke-consent", post(revoke_account_consent))
        .route("/account/delete", post(delete_account))
        .route("/account/mfa-setup", post(setup_account_mfa))
        .route("/approval/configure", post(configure_approval))
        .route("/approval/enable", post(enable_approval))
        .route("/approval/disable", post(disable_approval))
        .route("/approval/revoke-grant", post(revoke_approval_grant))
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantAccountResource {
    pub user_id: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantApprovalConfigResource {
    pub service_id: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantGrantResource {
    pub grant_id: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateAssistantAccountProfileRequest {
    pub action_request_id: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantAccountResponse {
    pub resource: AssistantAccountResource,
    pub replayed: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevokeAssistantConsentRequest {
    pub action_request_id: String,
    pub client_id: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteAssistantAccountRequest {
    pub action_request_id: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AssistantMfaSetupStage {
    Start,
    Confirm,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantMfaSetupRequest {
    pub action_request_id: String,
    pub stage: AssistantMfaSetupStage,
    pub factor_id: Option<String>,
    pub code: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMfaSetupResponse {
    pub resource: AssistantAccountResource,
    pub stage: AssistantMfaSetupStage,
    pub factor_id: Option<String>,
    pub setup_value: Option<String>,
    pub qr_code_url: Option<String>,
    pub recovery_values: Option<Vec<String>>,
    pub replayed: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigureAssistantApprovalRequest {
    pub action_request_id: String,
    pub service_id: String,
    pub approval_required: bool,
    #[schema(value_type = String)]
    pub approval_mode: ApprovalMode,
    #[schema(value_type = Vec<Object>)]
    pub rules: Vec<ApprovalRule>,
    #[schema(value_type = Option<String>)]
    pub default_effect: Option<ApprovalEffect>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToggleAssistantApprovalRequest {
    pub action_request_id: String,
    pub service_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantApprovalConfigResponse {
    pub resource: AssistantApprovalConfigResource,
    pub replayed: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevokeAssistantGrantRequest {
    pub action_request_id: String,
    pub grant_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantGrantResponse {
    pub resource: AssistantGrantResource,
    pub replayed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileFingerprint<'a> {
    action: &'static str,
    user_id: &'a str,
    display_name: Option<&'a str>,
    avatar_url: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConsentFingerprint<'a> {
    action: &'static str,
    user_id: &'a str,
    client_id: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExistingAccountFingerprint<'a> {
    action: &'static str,
    user_id: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MfaFingerprint<'a> {
    action: &'static str,
    user_id: &'a str,
    stage: AssistantMfaSetupStage,
    factor_id: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalConfigureFingerprint<'a> {
    action: &'static str,
    service_id: &'a str,
    effective_service_id: &'a str,
    approval_required: bool,
    approval_mode: &'a ApprovalMode,
    rules: &'a [ApprovalRule],
    default_effect: Option<&'a ApprovalEffect>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalToggleFingerprint<'a> {
    action: &'static str,
    service_id: &'a str,
    effective_service_id: &'a str,
    approval_required: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GrantFingerprint<'a> {
    action: &'static str,
    grant_id: &'a str,
    owner_user_id: &'a str,
}

struct NormalizedProfile {
    action_request_id: String,
    display_name: Option<String>,
    avatar_url: Option<String>,
}

fn parse_uuid(value: &str, field: &str) -> AppResult<String> {
    Uuid::parse_str(value.trim())
        .map(|id| id.to_string())
        .map_err(|_| AppError::ValidationError(format!("{field} must be a UUID")))
}

fn optional_trimmed(value: Option<String>) -> Option<String> {
    value.map(|raw| raw.trim().to_string())
}

fn reject_secret_shaped(field: &str, value: &str) -> AppResult<()> {
    if SECRET_SHAPED_VALUE.is_match(value) {
        return Err(AppError::ValidationError(format!(
            "{field} must not contain secret-shaped material"
        )));
    }
    Ok(())
}

fn normalize_profile(body: UpdateAssistantAccountProfileRequest) -> AppResult<NormalizedProfile> {
    let display_name = optional_trimmed(body.display_name);
    let avatar_url = optional_trimmed(body.avatar_url);
    if display_name.is_none() && avatar_url.is_none() {
        return Err(AppError::ValidationError(
            "account.profile_update requires displayName or avatarUrl".to_string(),
        ));
    }
    if let Some(value) = display_name.as_deref() {
        if value.is_empty() || value.len() > 200 {
            return Err(AppError::ValidationError(
                "displayName must contain 1 to 200 characters".to_string(),
            ));
        }
        reject_secret_shaped("displayName", value)?;
    }
    if let Some(value) = avatar_url.as_deref() {
        if value.is_empty() || value.len() > 2_048 {
            return Err(AppError::ValidationError(
                "avatarUrl must contain 1 to 2048 characters".to_string(),
            ));
        }
        let parsed = url::Url::parse(value)
            .map_err(|_| AppError::ValidationError("avatarUrl must be a valid URL".to_string()))?;
        if !matches!(parsed.scheme(), "http" | "https")
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.fragment().is_some()
        {
            return Err(AppError::ValidationError(
                "avatarUrl must be an HTTP(S) URL without userinfo or a fragment".to_string(),
            ));
        }
        reject_secret_shaped("avatarUrl", value)?;
    }
    Ok(NormalizedProfile {
        action_request_id: normalize_action_request_id(body.action_request_id)?,
        display_name,
        avatar_url,
    })
}

async fn require_user(state: &AppState, user_id: &str) -> AppResult<User> {
    state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))
}

async fn consent_exists(state: &AppState, user_id: &str, client_id: &str) -> AppResult<bool> {
    Ok(state
        .db
        .collection::<Consent>(CONSENTS)
        .find_one(doc! { "user_id": user_id, "client_id": client_id })
        .await?
        .is_some())
}

async fn effective_approval_service_id(
    state: &AppState,
    user_id: &str,
    service_id: &str,
) -> AppResult<String> {
    if let Some(service) = state
        .db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! {
            "_id": service_id,
            "user_id": user_id,
            "is_active": true,
        })
        .await?
    {
        return Ok(service.catalog_service_id.unwrap_or(service.id));
    }
    if state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": service_id, "is_active": true })
        .await?
        .is_some()
    {
        return Ok(service_id.to_string());
    }
    Err(AppError::NotFound("Service not found".to_string()))
}

async fn current_approval_config(
    state: &AppState,
    user_id: &str,
    effective_service_id: &str,
) -> AppResult<Option<ServiceApprovalConfig>> {
    Ok(state
        .db
        .collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
        .find_one(doc! {
            "user_id": user_id,
            "service_id": effective_service_id,
        })
        .await?)
}

fn config_matches(
    config: &ServiceApprovalConfig,
    approval_required: bool,
    approval_mode: Option<&ApprovalMode>,
    rules: Option<&[ApprovalRule]>,
    default_effect: Option<Option<&ApprovalEffect>>,
) -> bool {
    config.approval_required == approval_required
        && approval_mode.is_none_or(|mode| &config.approval_mode == mode)
        && rules.is_none_or(|rules| config.rules == rules)
        && default_effect.is_none_or(|effect| config.default_effect.as_ref() == effect)
}

async fn grant_for_actor(
    state: &AppState,
    actor: &str,
    grant_id: &str,
) -> AppResult<ApprovalGrant> {
    let grant = state
        .db
        .collection::<ApprovalGrant>(APPROVAL_GRANTS)
        .find_one(doc! { "_id": grant_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Grant not found".to_string()))?;
    if grant.user_id == actor {
        return Ok(grant);
    }

    let access = org_service::resolve_owner_access(&state.db, actor, &grant.user_id).await?;
    if !access.can_write() {
        return Err(AppError::NotFound("Grant not found".to_string()));
    }

    let mut service_ids = Vec::new();
    let mut cursor = state
        .db
        .collection::<UserService>(USER_SERVICES)
        .find(doc! {
            "user_id": &grant.user_id,
            "$or": [
                { "_id": &grant.service_id },
                { "catalog_service_id": &grant.service_id },
            ],
        })
        .await?;
    while let Some(service) = cursor.try_next().await? {
        service_ids.push(service.id);
    }
    if !access.allows_any_resource(&service_ids) {
        return Err(AppError::OrgRoleInsufficient(
            "your org role is scoped to other services and cannot revoke this grant".to_string(),
        ));
    }
    Ok(grant)
}

async fn update_account_profile(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<UpdateAssistantAccountProfileRequest>,
) -> AppResult<Json<AssistantAccountResponse>> {
    let request = normalize_profile(body)?;
    let user_id = auth_user.user_id.to_string();
    require_user(&state, &user_id).await?;
    let fingerprint = fingerprint_canonical(&ProfileFingerprint {
        action: ACCOUNT_PROFILE_UPDATE_ACTION,
        user_id: &user_id,
        display_name: request.display_name.as_deref(),
        avatar_url: request.avatar_url.as_deref(),
    })?;
    let receipt = reserve_or_replay(
        &state.db,
        &user_id,
        ACCOUNT_PROFILE_UPDATE_ACTION,
        &request.action_request_id,
        &fingerprint,
        user_id.clone(),
    )
    .await?;

    let replayed = match receipt {
        ReceiptOutcome::Replay(_) => true,
        ReceiptOutcome::InProgress(receipt) => {
            let user = require_user(&state, &user_id).await?;
            let already_applied = request
                .display_name
                .as_ref()
                .is_none_or(|value| user.display_name.as_ref() == Some(value))
                && request
                    .avatar_url
                    .as_ref()
                    .is_none_or(|value| user.avatar_url.as_ref() == Some(value));
            if !already_applied {
                return Err(in_progress_conflict());
            }
            mark_completed(&state.db, &receipt).await?;
            true
        }
        ReceiptOutcome::Reserved(receipt) => {
            let _ = users::update_me(
                State(state.clone()),
                auth_user,
                Json(users::UpdateProfileRequest {
                    display_name: request.display_name,
                    avatar_url: request.avatar_url,
                }),
            )
            .await?;
            mark_completed(&state.db, &receipt).await?;
            false
        }
    };

    Ok(Json(AssistantAccountResponse {
        resource: AssistantAccountResource { user_id },
        replayed,
    }))
}

async fn revoke_account_consent(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<RevokeAssistantConsentRequest>,
) -> AppResult<Json<AssistantAccountResponse>> {
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    let client_id = parse_uuid(&body.client_id, "clientId")?;
    let user_id = auth_user.user_id.to_string();
    require_user(&state, &user_id).await?;
    if !consent_exists(&state, &user_id, &client_id).await? {
        return Err(AppError::ConsentNotFound);
    }
    let fingerprint = fingerprint_canonical(&ConsentFingerprint {
        action: ACCOUNT_REVOKE_CONSENT_ACTION,
        user_id: &user_id,
        client_id: &client_id,
    })?;
    let receipt = reserve_or_replay(
        &state.db,
        &user_id,
        ACCOUNT_REVOKE_CONSENT_ACTION,
        &action_request_id,
        &fingerprint,
        user_id.clone(),
    )
    .await?;

    let replayed = match receipt {
        ReceiptOutcome::Replay(_) => true,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Reserved(receipt) => {
            let _ = consent::revoke_my_consent(State(state.clone()), auth_user, Path(client_id))
                .await?;
            mark_completed(&state.db, &receipt).await?;
            false
        }
    };

    Ok(Json(AssistantAccountResponse {
        resource: AssistantAccountResource { user_id },
        replayed,
    }))
}

async fn delete_account(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Json(body): Json<DeleteAssistantAccountRequest>,
) -> AppResult<Json<AssistantAccountResponse>> {
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    let user_id = auth_user.user_id.to_string();
    require_user(&state, &user_id).await?;
    let fingerprint = fingerprint_canonical(&ExistingAccountFingerprint {
        action: ACCOUNT_DELETE_ACTION,
        user_id: &user_id,
    })?;
    let receipt = reserve_or_replay(
        &state.db,
        &user_id,
        ACCOUNT_DELETE_ACTION,
        &action_request_id,
        &fingerprint,
        user_id.clone(),
    )
    .await?;

    let replayed = match receipt {
        ReceiptOutcome::Replay(_) => true,
        ReceiptOutcome::InProgress(receipt) => {
            if state
                .db
                .collection::<User>(USERS)
                .find_one(doc! { "_id": &user_id })
                .await?
                .is_some()
            {
                return Err(in_progress_conflict());
            }
            mark_completed(&state.db, &receipt).await?;
            true
        }
        ReceiptOutcome::Reserved(receipt) => {
            let _ =
                users::delete_me(State(state.clone()), auth_user, tele, HeaderMap::new()).await?;
            mark_completed(&state.db, &receipt).await?;
            false
        }
    };

    Ok(Json(AssistantAccountResponse {
        resource: AssistantAccountResource { user_id },
        replayed,
    }))
}

async fn setup_account_mfa(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Json(body): Json<AssistantMfaSetupRequest>,
) -> AppResult<Json<AssistantMfaSetupResponse>> {
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    let user_id = auth_user.user_id.to_string();
    let user = require_user(&state, &user_id).await?;

    match body.stage {
        AssistantMfaSetupStage::Start => {
            if body.factor_id.is_some() || body.code.is_some() {
                return Err(AppError::ValidationError(
                    "MFA start must not include factorId or code".to_string(),
                ));
            }
            if user.mfa_enabled {
                return Err(AppError::Conflict(
                    "MFA is already enabled for this account".to_string(),
                ));
            }
            let fingerprint = fingerprint_canonical(&MfaFingerprint {
                action: ACCOUNT_MFA_SETUP_START_ACTION,
                user_id: &user_id,
                stage: body.stage,
                factor_id: None,
            })?;
            let receipt = reserve_or_replay(
                &state.db,
                &user_id,
                ACCOUNT_MFA_SETUP_START_ACTION,
                &action_request_id,
                &fingerprint,
                user_id.clone(),
            )
            .await?;
            let receipt = match receipt {
                ReceiptOutcome::Reserved(receipt) => receipt,
                ReceiptOutcome::Replay(_) => {
                    return Err(AppError::Conflict(
                        "MFA setup material was already displayed; start a new assistant action"
                            .to_string(),
                    ));
                }
                ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
            };
            let Json(setup) = mfa::setup(State(state.clone()), auth_user, tele).await?;
            mark_completed(&state.db, &receipt).await?;
            Ok(Json(AssistantMfaSetupResponse {
                resource: AssistantAccountResource { user_id },
                stage: AssistantMfaSetupStage::Start,
                factor_id: Some(setup.factor_id),
                setup_value: Some(setup.secret),
                qr_code_url: Some(setup.qr_code_url),
                recovery_values: None,
                replayed: false,
            }))
        }
        AssistantMfaSetupStage::Confirm => {
            let factor_id = parse_uuid(
                body.factor_id.as_deref().ok_or_else(|| {
                    AppError::ValidationError("MFA confirm requires factorId".to_string())
                })?,
                "factorId",
            )?;
            let code = body.code.ok_or_else(|| {
                AppError::ValidationError("MFA confirm requires code".to_string())
            })?;
            if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(AppError::ValidationError(
                    "MFA code must contain exactly 6 digits".to_string(),
                ));
            }
            let factor = state
                .db
                .collection::<MfaFactor>(MFA_FACTORS)
                .find_one(doc! {
                    "_id": &factor_id,
                    "user_id": &user_id,
                    "factor_type": "totp",
                    "is_active": true,
                })
                .await?
                .ok_or_else(|| AppError::NotFound("MFA factor not found".to_string()))?;
            if factor.is_verified && !user.mfa_enabled {
                return Err(AppError::Conflict(
                    "MFA factor state is inconsistent".to_string(),
                ));
            }
            let fingerprint = fingerprint_canonical(&MfaFingerprint {
                action: ACCOUNT_MFA_SETUP_ACTION,
                user_id: &user_id,
                stage: body.stage,
                factor_id: Some(&factor_id),
            })?;
            let receipt = reserve_or_replay(
                &state.db,
                &user_id,
                ACCOUNT_MFA_SETUP_ACTION,
                &action_request_id,
                &fingerprint,
                user_id.clone(),
            )
            .await?;
            match receipt {
                ReceiptOutcome::Replay(_) => Ok(Json(AssistantMfaSetupResponse {
                    resource: AssistantAccountResource { user_id },
                    stage: AssistantMfaSetupStage::Confirm,
                    factor_id: Some(factor_id),
                    setup_value: None,
                    qr_code_url: None,
                    recovery_values: None,
                    replayed: true,
                })),
                ReceiptOutcome::InProgress(receipt) => {
                    let user = require_user(&state, &user_id).await?;
                    if !user.mfa_enabled {
                        return Err(in_progress_conflict());
                    }
                    mark_completed(&state.db, &receipt).await?;
                    Ok(Json(AssistantMfaSetupResponse {
                        resource: AssistantAccountResource { user_id },
                        stage: AssistantMfaSetupStage::Confirm,
                        factor_id: Some(factor_id),
                        setup_value: None,
                        qr_code_url: None,
                        recovery_values: None,
                        replayed: true,
                    }))
                }
                ReceiptOutcome::Reserved(receipt) => {
                    let Json(confirmed) = mfa::confirm(
                        State(state.clone()),
                        auth_user,
                        tele,
                        Json(mfa::MfaConfirmRequest { code }),
                    )
                    .await?;
                    mark_completed(&state.db, &receipt).await?;
                    Ok(Json(AssistantMfaSetupResponse {
                        resource: AssistantAccountResource { user_id },
                        stage: AssistantMfaSetupStage::Confirm,
                        factor_id: Some(factor_id),
                        setup_value: None,
                        qr_code_url: None,
                        recovery_values: Some(confirmed.recovery_codes),
                        replayed: false,
                    }))
                }
            }
        }
    }
}

async fn set_approval_config(
    state: AppState,
    auth_user: AuthUser,
    tele: TelemetryContext,
    action_request_id: String,
    service_id: String,
    action: &'static str,
    approval_required: bool,
    approval_mode: Option<ApprovalMode>,
    rules: Option<Vec<ApprovalRule>>,
    default_effect: Option<Option<ApprovalEffect>>,
) -> AppResult<Json<AssistantApprovalConfigResponse>> {
    let action_request_id = normalize_action_request_id(action_request_id)?;
    let service_id = parse_uuid(&service_id, "serviceId")?;
    let user_id = auth_user.user_id.to_string();
    let effective_service_id = effective_approval_service_id(&state, &user_id, &service_id).await?;
    if let Some(ref rules) = rules {
        crate::services::approval_policy::validate_rules(rules)
            .map_err(AppError::ValidationError)?;
    }

    let fingerprint = if action == APPROVAL_CONFIGURE_ACTION {
        fingerprint_canonical(&ApprovalConfigureFingerprint {
            action,
            service_id: &service_id,
            effective_service_id: &effective_service_id,
            approval_required,
            approval_mode: approval_mode.as_ref().expect("configure mode"),
            rules: rules.as_deref().expect("configure rules"),
            default_effect: default_effect
                .as_ref()
                .expect("configure default effect")
                .as_ref(),
        })?
    } else {
        fingerprint_canonical(&ApprovalToggleFingerprint {
            action,
            service_id: &service_id,
            effective_service_id: &effective_service_id,
            approval_required,
        })?
    };
    let receipt = reserve_or_replay(
        &state.db,
        &user_id,
        action,
        &action_request_id,
        &fingerprint,
        service_id.clone(),
    )
    .await?;

    let replayed = match receipt {
        ReceiptOutcome::Replay(_) => true,
        ReceiptOutcome::InProgress(receipt) => {
            let config = current_approval_config(&state, &user_id, &effective_service_id)
                .await?
                .ok_or_else(in_progress_conflict)?;
            if !config_matches(
                &config,
                approval_required,
                approval_mode.as_ref(),
                rules.as_deref(),
                default_effect.as_ref().map(|value| value.as_ref()),
            ) {
                return Err(in_progress_conflict());
            }
            mark_completed(&state.db, &receipt).await?;
            true
        }
        ReceiptOutcome::Reserved(receipt) => {
            let _ = approvals::set_service_config(
                State(state.clone()),
                auth_user,
                tele,
                Path(service_id.clone()),
                Query(approvals::ServiceApprovalConfigQuery::default()),
                Json(approvals::SetServiceApprovalConfigRequest {
                    approval_required: Some(approval_required),
                    approval_mode,
                    rules,
                    default_effect: default_effect.flatten(),
                }),
            )
            .await?;
            mark_completed(&state.db, &receipt).await?;
            false
        }
    };

    Ok(Json(AssistantApprovalConfigResponse {
        resource: AssistantApprovalConfigResource { service_id },
        replayed,
    }))
}

async fn configure_approval(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Json(body): Json<ConfigureAssistantApprovalRequest>,
) -> AppResult<Json<AssistantApprovalConfigResponse>> {
    set_approval_config(
        state,
        auth_user,
        tele,
        body.action_request_id,
        body.service_id,
        APPROVAL_CONFIGURE_ACTION,
        body.approval_required,
        Some(body.approval_mode),
        Some(body.rules),
        Some(body.default_effect),
    )
    .await
}

async fn enable_approval(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Json(body): Json<ToggleAssistantApprovalRequest>,
) -> AppResult<Json<AssistantApprovalConfigResponse>> {
    set_approval_config(
        state,
        auth_user,
        tele,
        body.action_request_id,
        body.service_id,
        APPROVAL_ENABLE_ACTION,
        true,
        None,
        None,
        None,
    )
    .await
}

async fn disable_approval(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Json(body): Json<ToggleAssistantApprovalRequest>,
) -> AppResult<Json<AssistantApprovalConfigResponse>> {
    set_approval_config(
        state,
        auth_user,
        tele,
        body.action_request_id,
        body.service_id,
        APPROVAL_DISABLE_ACTION,
        false,
        None,
        None,
        None,
    )
    .await
}

async fn revoke_approval_grant(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Json(body): Json<RevokeAssistantGrantRequest>,
) -> AppResult<Json<AssistantGrantResponse>> {
    let action_request_id = normalize_action_request_id(body.action_request_id)?;
    let grant_id = parse_uuid(&body.grant_id, "grantId")?;
    let actor = auth_user.user_id.to_string();
    let grant = grant_for_actor(&state, &actor, &grant_id).await?;
    if grant.revoked {
        return Err(AppError::NotFound("Active grant not found".to_string()));
    }
    let fingerprint = fingerprint_canonical(&GrantFingerprint {
        action: APPROVAL_REVOKE_GRANT_ACTION,
        grant_id: &grant_id,
        owner_user_id: &grant.user_id,
    })?;
    let receipt = reserve_or_replay(
        &state.db,
        &actor,
        APPROVAL_REVOKE_GRANT_ACTION,
        &action_request_id,
        &fingerprint,
        grant_id.clone(),
    )
    .await?;

    let replayed = match receipt {
        ReceiptOutcome::Replay(_) => true,
        ReceiptOutcome::InProgress(receipt) => {
            let current = grant_for_actor(&state, &actor, &grant_id).await?;
            if !current.revoked {
                return Err(in_progress_conflict());
            }
            mark_completed(&state.db, &receipt).await?;
            true
        }
        ReceiptOutcome::Reserved(receipt) => {
            approval_service::revoke_grant(&state.db, &grant.user_id, &grant_id).await?;
            crate::services::audit_service::log_for_user(
                state.db.clone(),
                &auth_user,
                "approval_grant_revoked",
                Some(serde_json::json!({
                    "grant_id": grant_id,
                    "owner_user_id": grant.user_id,
                    "assistant_action": true,
                })),
            );
            let _ = tele;
            mark_completed(&state.db, &receipt).await?;
            false
        }
    };

    Ok(Json(AssistantGrantResponse {
        resource: AssistantGrantResource { grant_id },
        replayed,
    }))
}
