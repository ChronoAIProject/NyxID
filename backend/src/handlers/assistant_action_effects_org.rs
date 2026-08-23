//! Wave-4 organization and account-family assistant action effects.
//!
//! The browser is the only place that collects confirmation values and
//! one-time material. Every committing path reserves a durable receipt before
//! the mutation, fingerprints all non-secret semantic content, and treats a
//! pending receipt as a distinct state that may only be resumed after proving
//! the requested state already exists.

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
use crate::handlers::{
    admin_service_accounts, developer_apps, keys, notifications, orgs, user_api_keys_external,
};
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
const ORG_CREATE_ACTION: &str = "org.create";
const ORG_UPDATE_ACTION: &str = "org.update";
const ORG_DELETE_ACTION: &str = "org.delete";
const ORG_MEMBER_ADD_ACTION: &str = "org.member_add";
const ORG_MEMBER_REMOVE_ACTION: &str = "org.member_remove";
const ORG_MEMBER_UPDATE_ROLE_ACTION: &str = "org.member_update_role";
const ORG_INVITE_ACTION: &str = "org.invite";
const ORG_SET_PRIMARY_ACTION: &str = "org.set_primary";
const SERVICE_ACCOUNT_CREATE_ACTION: &str = "service_account.create";
const SERVICE_ACCOUNT_UPDATE_ACTION: &str = "service_account.update";
const SERVICE_ACCOUNT_DELETE_ACTION: &str = "service_account.delete";
const SERVICE_ACCOUNT_ROTATE_SECRET_ACTION: &str = "service_account.rotate_secret";
const SERVICE_ACCOUNT_REVOKE_TOKENS_ACTION: &str = "service_account.revoke_tokens";
const DEVELOPER_APP_CREATE_ACTION: &str = "developer_app.create";
const DEVELOPER_APP_UPDATE_ACTION: &str = "developer_app.update";
const DEVELOPER_APP_DELETE_ACTION: &str = "developer_app.delete";
const DEVELOPER_APP_ROTATE_SECRET_ACTION: &str = "developer_app.rotate_secret";
const NOTIFICATIONS_UPDATE_ACTION: &str = "notifications.update";
const NOTIFICATIONS_TELEGRAM_LINK_ACTION: &str = "notifications.telegram_link";
const NOTIFICATIONS_TELEGRAM_DISCONNECT_ACTION: &str = "notifications.telegram_disconnect";
const EXTERNAL_KEY_ADD_GCP_ACTION: &str = "external_key.add_gcp_service_account";
const OPENCLAW_CONNECT_ACTION: &str = "openclaw.connect";

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
        .route("/org/create", post(create_org_action))
        .route("/org/update", post(update_org_action))
        .route("/org/delete", post(delete_org_action))
        .route("/org/member-add", post(add_org_member_action))
        .route("/org/member-remove", post(remove_org_member_action))
        .route(
            "/org/member-update-role",
            post(update_org_member_role_action),
        )
        .route("/org/invite", post(invite_org_action))
        .route("/org/set-primary", post(set_primary_org_action))
        .route(
            "/service-account/create",
            post(create_service_account_action),
        )
        .route(
            "/service-account/update",
            post(update_service_account_action),
        )
        .route(
            "/service-account/delete",
            post(delete_service_account_action),
        )
        .route(
            "/service-account/rotate-secret",
            post(rotate_service_account_secret_action),
        )
        .route(
            "/service-account/revoke-tokens",
            post(revoke_service_account_tokens_action),
        )
        .route("/developer-app/create", post(create_developer_app_action))
        .route("/developer-app/update", post(update_developer_app_action))
        .route("/developer-app/delete", post(delete_developer_app_action))
        .route(
            "/developer-app/rotate-secret",
            post(rotate_developer_app_secret_action),
        )
        .route("/notifications/update", post(update_notifications_action))
        .route("/notifications/telegram-link", post(link_telegram_action))
        .route(
            "/notifications/telegram-disconnect",
            post(disconnect_telegram_action),
        )
        .route(
            "/external-key/add-gcp-service-account",
            post(add_gcp_service_account_action),
        )
        .route("/openclaw/connect", post(connect_openclaw_action))
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
    /// Typed confirmation of the account's own email address.
    ///
    /// The browser also asks for this, but a browser-side check is not a
    /// control: the effect route is mounted and reachable, so the server has
    /// to verify the confirmation itself for it to mean anything.
    pub confirm_email: String,
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

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantOrgResource {
    pub org_id: String,
}
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantNotificationBindingResource {
    pub binding_id: String,
}
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantServiceAccountResource {
    pub service_account_id: String,
}
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantDeveloperAppResource {
    pub client_id: String,
}
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantExternalKeyResource {
    pub external_key_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantOrgResponse {
    pub resource: AssistantOrgResource,
    pub replayed: bool,
}
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantNotificationResponse {
    pub resource: AssistantNotificationBindingResource,
    pub replayed: bool,
}
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantTelegramLinkResponse {
    pub resource: AssistantNotificationBindingResource,
    pub replayed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in_secs: Option<u32>,
}
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantServiceAccountResponse {
    pub resource: AssistantServiceAccountResource,
    pub replayed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
}
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantDeveloperAppResponse {
    pub resource: AssistantDeveloperAppResource,
    pub replayed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
}
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantExternalKeyResponse {
    pub resource: AssistantExternalKeyResource,
    pub replayed: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantUserServiceResource {
    pub user_service_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantUserServiceResponse {
    pub resource: AssistantUserServiceResource,
    pub replayed: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantOrgCreateRequest {
    pub action_request_id: String,
    pub display_name: String,
    pub contact_email: Option<String>,
    pub avatar_url: Option<String>,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantOrgUpdateRequest {
    pub action_request_id: String,
    pub org_id: String,
    pub display_name: Option<String>,
    pub slug: Option<String>,
    pub contact_email: Option<String>,
    pub avatar_url: Option<String>,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantOrgIdRequest {
    pub action_request_id: String,
    pub org_id: String,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantOrgMemberAddRequest {
    pub action_request_id: String,
    pub org_id: String,
    pub user_id: String,
    pub role: orgs::OrgRoleWire,
    pub allowed_service_ids: Option<Vec<String>>,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantOrgMemberRequest {
    pub action_request_id: String,
    pub org_id: String,
    pub member_id: String,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantOrgMemberRoleRequest {
    pub action_request_id: String,
    pub org_id: String,
    pub member_id: String,
    pub role: orgs::OrgRoleWire,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantOrgInviteRequest {
    pub action_request_id: String,
    pub org_id: String,
    pub role: orgs::OrgRoleWire,
    pub allowed_service_ids: Option<Vec<String>>,
    pub ttl_hours: Option<i64>,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantServiceAccountCreateRequest {
    pub action_request_id: String,
    pub name: String,
    pub description: Option<String>,
    pub allowed_scopes: Option<String>,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantServiceAccountRequest {
    pub action_request_id: String,
    pub service_account_id: String,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantServiceAccountUpdateRequest {
    pub action_request_id: String,
    pub service_account_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantDeveloperAppCreateRequest {
    pub action_request_id: String,
    pub name: String,
    pub redirect_uris: Vec<String>,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantDeveloperAppRequest {
    pub action_request_id: String,
    pub client_id: String,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantDeveloperAppUpdateRequest {
    pub action_request_id: String,
    pub client_id: String,
    pub name: Option<String>,
    pub redirect_uris: Option<Vec<String>>,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantNotificationsUpdateRequest {
    pub action_request_id: String,
    pub telegram_enabled: Option<bool>,
    pub approval_required: Option<bool>,
    pub approval_timeout_secs: Option<u32>,
    pub grant_expiry_days: Option<u32>,
    pub push_enabled: Option<bool>,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantActionRequestId {
    pub action_request_id: String,
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantGcpCreateRequest {
    pub action_request_id: String,
    pub label: Option<String>,
    pub key_json: String,
    pub scopes: Option<String>,
    pub service_slugs: Option<Vec<String>>,
    pub target_org_id: Option<String>,
}
impl std::fmt::Debug for AssistantGcpCreateRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistantGcpCreateRequest")
            .field("action_request_id", &self.action_request_id)
            .field("label", &self.label)
            .field("key_json", &"[REDACTED]")
            .finish()
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantOpenClawConnectRequest {
    pub action_request_id: String,
    pub gateway_url: String,
    pub credential: String,
    pub label: Option<String>,
}
impl std::fmt::Debug for AssistantOpenClawConnectRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistantOpenClawConnectRequest")
            .field("action_request_id", &self.action_request_id)
            .field("gateway_url", &self.gateway_url)
            .field("credential", &"[REDACTED]")
            .finish()
    }
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

/// The confirmation is semantic request content, so it is fingerprinted:
/// reusing one `actionRequestId` with a different confirmation must conflict
/// rather than replay.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfirmedAccountDeleteFingerprint<'a> {
    action: &'static str,
    user_id: &'a str,
    confirm_email: &'a str,
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
    let user = require_user(&state, &user_id).await?;

    // Verify the confirmation BEFORE reserving, so a wrong or missing one
    // leaves no receipt behind and the user can simply retype it.
    let confirm_email = body.confirm_email.trim().to_ascii_lowercase();
    if confirm_email.is_empty() || confirm_email != user.email.trim().to_ascii_lowercase() {
        return Err(AppError::ValidationError(
            "confirmEmail must match the account email exactly".to_string(),
        ));
    }

    let fingerprint = fingerprint_canonical(&ConfirmedAccountDeleteFingerprint {
        action: ACCOUNT_DELETE_ACTION,
        user_id: &user_id,
        confirm_email: &confirm_email,
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

// ---------------------------------------------------------------------------
// Wave-4 effects
// ---------------------------------------------------------------------------

fn action_fingerprint(action: &'static str, value: serde_json::Value) -> AppResult<String> {
    fingerprint_canonical(&serde_json::json!({ "action": action, "request": value }))
}

fn service_account_create_fingerprint(
    name: &str,
    description: Option<&str>,
    allowed_scopes: Option<&str>,
) -> AppResult<String> {
    action_fingerprint(
        SERVICE_ACCOUNT_CREATE_ACTION,
        serde_json::json!({"name":name,"description":description,"allowedScopes":allowed_scopes}),
    )
}

fn gcp_create_fingerprint(body: &AssistantGcpCreateRequest) -> AppResult<String> {
    action_fingerprint(
        EXTERNAL_KEY_ADD_GCP_ACTION,
        serde_json::json!({
            "label": body.label.as_deref(),
            // Keyed, not plain: the receipt is stored, so an unkeyed digest of
            // caller-supplied material is an offline oracle against a database
            // snapshot. Identifiers below stay plain -- they are not secret.
            "keyJson": crate::services::assistant_action_receipts::fingerprint_sensitive_material(
                body.key_json.as_str(),
            ),
            "scopes": body.scopes.as_deref(),
            "serviceSlugs": body.service_slugs.as_deref(),
            "targetOrgId": body.target_org_id.as_deref(),
        }),
    )
}

fn openclaw_connect_fingerprint(
    gateway_url: &str,
    credential: &str,
    label: Option<&str>,
) -> AppResult<String> {
    action_fingerprint(
        OPENCLAW_CONNECT_ACTION,
        serde_json::json!({
            "gatewayUrl": gateway_url,
            "credential": crate::services::assistant_action_receipts::fingerprint_sensitive_material(
                credential,
            ),
            "label": label,
        }),
    )
}

async fn complete_wave4(
    state: &AppState,
    receipt: &crate::models::assistant_action_receipt::AssistantActionReceipt,
    resource_id: &str,
) -> AppResult<()> {
    // Create-shaped actions reserve an identity before the service layer picks
    // its UUID. Persist the authoritative id before marking the receipt done,
    // so an exact retry can return the same typed reference.
    state
        .db
        .collection::<crate::models::assistant_action_receipt::AssistantActionReceipt>(
            crate::models::assistant_action_receipt::COLLECTION_NAME,
        )
        .update_one(
            doc! { "_id": &receipt.id },
            doc! { "$set": { "resource_id": resource_id } },
        )
        .await?;
    mark_completed(&state.db, receipt).await
}

fn replayed_resource(outcome: &ReceiptOutcome) -> Option<(String, bool)> {
    match outcome {
        ReceiptOutcome::Replay(receipt) => Some((receipt.resource_id.clone(), true)),
        _ => None,
    }
}

async fn create_org_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantOrgCreateRequest>,
) -> AppResult<Json<AssistantOrgResponse>> {
    let actor = auth_user.user_id.to_string();
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = action_fingerprint(
        ORG_CREATE_ACTION,
        serde_json::json!({
            "displayName": body.display_name,
            "contactEmail": body.contact_email,
            "avatarUrl": body.avatar_url,
        }),
    )?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        ORG_CREATE_ACTION,
        &request_id,
        &fp,
        Uuid::new_v4().to_string(),
    )
    .await?;
    if let Some((org_id, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantOrgResponse {
            resource: AssistantOrgResource { org_id },
            replayed,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let (_, Json(org)) = orgs::create_org(
        State(state.clone()),
        auth_user,
        Json(orgs::CreateOrgRequest {
            display_name: body.display_name,
            contact_email: body.contact_email,
            avatar_url: body.avatar_url,
        }),
    )
    .await?;
    complete_wave4(&state, &receipt, &org.id).await?;
    Ok(Json(AssistantOrgResponse {
        resource: AssistantOrgResource { org_id: org.id },
        replayed: false,
    }))
}

async fn update_org_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantOrgUpdateRequest>,
) -> AppResult<Json<AssistantOrgResponse>> {
    let actor = auth_user.user_id.to_string();
    let org_id = parse_uuid(&body.org_id, "orgId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = action_fingerprint(
        ORG_UPDATE_ACTION,
        serde_json::json!({"orgId":org_id,"displayName":body.display_name,"slug":body.slug,"contactEmail":body.contact_email,"avatarUrl":body.avatar_url}),
    )?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        ORG_UPDATE_ACTION,
        &request_id,
        &fp,
        org_id.clone(),
    )
    .await?;
    if let Some((_, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantOrgResponse {
            resource: AssistantOrgResource { org_id },
            replayed,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let _ = orgs::update_org(
        State(state.clone()),
        auth_user,
        Path(org_id.clone()),
        Json(orgs::UpdateOrgRequest {
            display_name: body.display_name,
            slug: body.slug,
            avatar_url: body.avatar_url,
            contact_email: body.contact_email,
            remote_credential_integrity_verification_opt_out: None,
        }),
    )
    .await?;
    complete_wave4(&state, &receipt, &org_id).await?;
    Ok(Json(AssistantOrgResponse {
        resource: AssistantOrgResource { org_id },
        replayed: false,
    }))
}

async fn delete_org_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantOrgIdRequest>,
) -> AppResult<Json<AssistantOrgResponse>> {
    let actor = auth_user.user_id.to_string();
    let org_id = parse_uuid(&body.org_id, "orgId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = action_fingerprint(ORG_DELETE_ACTION, serde_json::json!({"orgId":org_id}))?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        ORG_DELETE_ACTION,
        &request_id,
        &fp,
        org_id.clone(),
    )
    .await?;
    if let Some((_, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantOrgResponse {
            resource: AssistantOrgResource { org_id },
            replayed,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    orgs::delete_org(State(state.clone()), auth_user, Path(org_id.clone())).await?;
    complete_wave4(&state, &receipt, &org_id).await?;
    Ok(Json(AssistantOrgResponse {
        resource: AssistantOrgResource { org_id },
        replayed: false,
    }))
}

async fn add_org_member_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantOrgMemberAddRequest>,
) -> AppResult<Json<AssistantOrgResponse>> {
    let actor = auth_user.user_id.to_string();
    let org_id = parse_uuid(&body.org_id, "orgId")?;
    let user_id = parse_uuid(&body.user_id, "userId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = action_fingerprint(
        ORG_MEMBER_ADD_ACTION,
        serde_json::json!({"orgId":org_id,"userId":user_id,"role":body.role,"allowedServiceIds":body.allowed_service_ids}),
    )?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        ORG_MEMBER_ADD_ACTION,
        &request_id,
        &fp,
        org_id.clone(),
    )
    .await?;
    if let Some((_, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantOrgResponse {
            resource: AssistantOrgResource { org_id },
            replayed,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let _ = orgs::add_member(
        State(state.clone()),
        auth_user,
        Path(org_id.clone()),
        Json(orgs::AddMemberRequest {
            user_id,
            role: body.role,
            scope_source: None,
            allowed_service_ids: body.allowed_service_ids,
        }),
    )
    .await?;
    complete_wave4(&state, &receipt, &org_id).await?;
    Ok(Json(AssistantOrgResponse {
        resource: AssistantOrgResource { org_id },
        replayed: false,
    }))
}

async fn remove_org_member_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantOrgMemberRequest>,
) -> AppResult<Json<AssistantOrgResponse>> {
    let actor = auth_user.user_id.to_string();
    let org_id = parse_uuid(&body.org_id, "orgId")?;
    let member_id = parse_uuid(&body.member_id, "memberId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = action_fingerprint(
        ORG_MEMBER_REMOVE_ACTION,
        serde_json::json!({"orgId":org_id,"memberId":member_id}),
    )?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        ORG_MEMBER_REMOVE_ACTION,
        &request_id,
        &fp,
        org_id.clone(),
    )
    .await?;
    if let Some((_, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantOrgResponse {
            resource: AssistantOrgResource { org_id },
            replayed,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    orgs::remove_member(
        State(state.clone()),
        auth_user,
        Path((org_id.clone(), member_id)),
    )
    .await?;
    complete_wave4(&state, &receipt, &org_id).await?;
    Ok(Json(AssistantOrgResponse {
        resource: AssistantOrgResource { org_id },
        replayed: false,
    }))
}

async fn update_org_member_role_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantOrgMemberRoleRequest>,
) -> AppResult<Json<AssistantOrgResponse>> {
    let actor = auth_user.user_id.to_string();
    let org_id = parse_uuid(&body.org_id, "orgId")?;
    let member_id = parse_uuid(&body.member_id, "memberId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = action_fingerprint(
        ORG_MEMBER_UPDATE_ROLE_ACTION,
        serde_json::json!({"orgId":org_id,"memberId":member_id,"role":body.role}),
    )?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        ORG_MEMBER_UPDATE_ROLE_ACTION,
        &request_id,
        &fp,
        org_id.clone(),
    )
    .await?;
    if let Some((_, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantOrgResponse {
            resource: AssistantOrgResource { org_id },
            replayed,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let _ = orgs::update_member(
        State(state.clone()),
        auth_user,
        Path((org_id.clone(), member_id)),
        Json(orgs::UpdateMemberRequest {
            role: Some(body.role),
            scope_source: None,
            allowed_service_ids: None,
        }),
    )
    .await?;
    complete_wave4(&state, &receipt, &org_id).await?;
    Ok(Json(AssistantOrgResponse {
        resource: AssistantOrgResource { org_id },
        replayed: false,
    }))
}

async fn invite_org_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantOrgInviteRequest>,
) -> AppResult<Json<AssistantOrgResponse>> {
    let actor = auth_user.user_id.to_string();
    let org_id = parse_uuid(&body.org_id, "orgId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = action_fingerprint(
        ORG_INVITE_ACTION,
        serde_json::json!({"orgId":org_id,"role":body.role,"allowedServiceIds":body.allowed_service_ids,"ttlHours":body.ttl_hours}),
    )?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        ORG_INVITE_ACTION,
        &request_id,
        &fp,
        org_id.clone(),
    )
    .await?;
    if let Some((_, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantOrgResponse {
            resource: AssistantOrgResource { org_id },
            replayed,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let _ = orgs::create_invite(
        State(state.clone()),
        auth_user,
        Path(org_id.clone()),
        Json(orgs::CreateInviteRequest {
            role: body.role,
            scope_source: None,
            allowed_service_ids: body.allowed_service_ids,
            ttl_hours: body.ttl_hours,
        }),
    )
    .await?;
    complete_wave4(&state, &receipt, &org_id).await?;
    Ok(Json(AssistantOrgResponse {
        resource: AssistantOrgResource { org_id },
        replayed: false,
    }))
}

async fn set_primary_org_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantOrgIdRequest>,
) -> AppResult<Json<AssistantOrgResponse>> {
    let actor = auth_user.user_id.to_string();
    let org_id = parse_uuid(&body.org_id, "orgId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = action_fingerprint(ORG_SET_PRIMARY_ACTION, serde_json::json!({"orgId":org_id}))?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        ORG_SET_PRIMARY_ACTION,
        &request_id,
        &fp,
        org_id.clone(),
    )
    .await?;
    if let Some((_, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantOrgResponse {
            resource: AssistantOrgResource { org_id },
            replayed,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let _ = orgs::set_primary_org(
        State(state.clone()),
        auth_user,
        Json(orgs::SetPrimaryOrgRequest {
            primary_org_id: Some(org_id.clone()),
        }),
    )
    .await?;
    complete_wave4(&state, &receipt, &org_id).await?;
    Ok(Json(AssistantOrgResponse {
        resource: AssistantOrgResource { org_id },
        replayed: false,
    }))
}

async fn create_service_account_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantServiceAccountCreateRequest>,
) -> AppResult<Json<AssistantServiceAccountResponse>> {
    let actor = auth_user.user_id.to_string();
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = service_account_create_fingerprint(
        &body.name,
        body.description.as_deref(),
        body.allowed_scopes.as_deref(),
    )?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        SERVICE_ACCOUNT_CREATE_ACTION,
        &request_id,
        &fp,
        Uuid::new_v4().to_string(),
    )
    .await?;
    if let Some((id, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantServiceAccountResponse {
            resource: AssistantServiceAccountResource {
                service_account_id: id,
            },
            replayed,
            client_secret: None,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let Json(created) = admin_service_accounts::create_service_account(
        State(state.clone()),
        auth_user,
        TelemetryContext::default(),
        HeaderMap::new(),
        Json(admin_service_accounts::CreateServiceAccountRequest {
            name: body.name,
            description: body.description,
            allowed_scopes: body.allowed_scopes.unwrap_or_else(|| "proxy".to_string()),
            role_ids: None,
            rate_limit_override: None,
            target_org_id: None,
        }),
    )
    .await?;
    complete_wave4(&state, &receipt, &created.id).await?;
    Ok(Json(AssistantServiceAccountResponse {
        resource: AssistantServiceAccountResource {
            service_account_id: created.id,
        },
        replayed: false,
        client_secret: Some(created.client_secret),
    }))
}

async fn update_service_account_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantServiceAccountUpdateRequest>,
) -> AppResult<Json<AssistantServiceAccountResponse>> {
    let actor = auth_user.user_id.to_string();
    let id = parse_uuid(&body.service_account_id, "serviceAccountId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = action_fingerprint(
        SERVICE_ACCOUNT_UPDATE_ACTION,
        serde_json::json!({"id":id,"name":body.name,"description":body.description}),
    )?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        SERVICE_ACCOUNT_UPDATE_ACTION,
        &request_id,
        &fp,
        id.clone(),
    )
    .await?;
    if let Some((_, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantServiceAccountResponse {
            resource: AssistantServiceAccountResource {
                service_account_id: id,
            },
            replayed,
            client_secret: None,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let _ = admin_service_accounts::update_service_account(
        State(state.clone()),
        auth_user,
        HeaderMap::new(),
        Path(id.clone()),
        Json(admin_service_accounts::UpdateServiceAccountRequest {
            name: body.name,
            description: body.description,
            allowed_scopes: None,
            role_ids: None,
            rate_limit_override: None,
            is_active: None,
        }),
    )
    .await?;
    complete_wave4(&state, &receipt, &id).await?;
    Ok(Json(AssistantServiceAccountResponse {
        resource: AssistantServiceAccountResource {
            service_account_id: id,
        },
        replayed: false,
        client_secret: None,
    }))
}

async fn delete_service_account_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantServiceAccountRequest>,
) -> AppResult<Json<AssistantServiceAccountResponse>> {
    let actor = auth_user.user_id.to_string();
    let id = parse_uuid(&body.service_account_id, "serviceAccountId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = action_fingerprint(SERVICE_ACCOUNT_DELETE_ACTION, serde_json::json!({"id":id}))?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        SERVICE_ACCOUNT_DELETE_ACTION,
        &request_id,
        &fp,
        id.clone(),
    )
    .await?;
    if let Some((_, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantServiceAccountResponse {
            resource: AssistantServiceAccountResource {
                service_account_id: id,
            },
            replayed,
            client_secret: None,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let _ = admin_service_accounts::delete_service_account(
        State(state.clone()),
        auth_user,
        TelemetryContext::default(),
        HeaderMap::new(),
        Path(id.clone()),
    )
    .await?;
    complete_wave4(&state, &receipt, &id).await?;
    Ok(Json(AssistantServiceAccountResponse {
        resource: AssistantServiceAccountResource {
            service_account_id: id,
        },
        replayed: false,
        client_secret: None,
    }))
}

async fn rotate_service_account_secret_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantServiceAccountRequest>,
) -> AppResult<Json<AssistantServiceAccountResponse>> {
    let actor = auth_user.user_id.to_string();
    let id = parse_uuid(&body.service_account_id, "serviceAccountId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = action_fingerprint(
        SERVICE_ACCOUNT_ROTATE_SECRET_ACTION,
        serde_json::json!({"id":id}),
    )?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        SERVICE_ACCOUNT_ROTATE_SECRET_ACTION,
        &request_id,
        &fp,
        id.clone(),
    )
    .await?;
    if let Some((_, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantServiceAccountResponse {
            resource: AssistantServiceAccountResource {
                service_account_id: id,
            },
            replayed,
            client_secret: None,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let rotated = admin_service_accounts::rotate_secret(
        State(state.clone()),
        auth_user,
        TelemetryContext::default(),
        HeaderMap::new(),
        Path(id.clone()),
    )
    .await?;
    let rotated_secret = rotated.0.client_secret;
    complete_wave4(&state, &receipt, &id).await?;
    Ok(Json(AssistantServiceAccountResponse {
        resource: AssistantServiceAccountResource {
            service_account_id: id,
        },
        replayed: false,
        client_secret: Some(rotated_secret),
    }))
}

async fn revoke_service_account_tokens_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantServiceAccountRequest>,
) -> AppResult<Json<AssistantServiceAccountResponse>> {
    let actor = auth_user.user_id.to_string();
    let id = parse_uuid(&body.service_account_id, "serviceAccountId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = action_fingerprint(
        SERVICE_ACCOUNT_REVOKE_TOKENS_ACTION,
        serde_json::json!({"id":id}),
    )?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        SERVICE_ACCOUNT_REVOKE_TOKENS_ACTION,
        &request_id,
        &fp,
        id.clone(),
    )
    .await?;
    if let Some((_, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantServiceAccountResponse {
            resource: AssistantServiceAccountResource {
                service_account_id: id,
            },
            replayed,
            client_secret: None,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let _ = admin_service_accounts::revoke_tokens(
        State(state.clone()),
        auth_user,
        HeaderMap::new(),
        Path(id.clone()),
    )
    .await?;
    complete_wave4(&state, &receipt, &id).await?;
    Ok(Json(AssistantServiceAccountResponse {
        resource: AssistantServiceAccountResource {
            service_account_id: id,
        },
        replayed: false,
        client_secret: None,
    }))
}

async fn create_developer_app_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantDeveloperAppCreateRequest>,
) -> AppResult<Json<AssistantDeveloperAppResponse>> {
    let actor = auth_user.user_id.to_string();
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = action_fingerprint(
        DEVELOPER_APP_CREATE_ACTION,
        serde_json::json!({"name":body.name,"redirectUris":body.redirect_uris}),
    )?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        DEVELOPER_APP_CREATE_ACTION,
        &request_id,
        &fp,
        Uuid::new_v4().to_string(),
    )
    .await?;
    if let Some((id, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantDeveloperAppResponse {
            resource: AssistantDeveloperAppResource { client_id: id },
            replayed,
            client_secret: None,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let Json(created) = developer_apps::create_my_oauth_client(
        State(state.clone()),
        auth_user,
        TelemetryContext::default(),
        Json(developer_apps::CreateDeveloperOAuthClientRequest {
            name: body.name,
            redirect_uris: body.redirect_uris,
            client_type: Some("confidential".to_string()),
            delegation_scopes: None,
            broker_capability_enabled: None,
            revocation_webhook_url: None,
            revocation_webhook_secret: None,
            allowed_scopes: None,
            target_org_id: None,
            default_service_catalog_slugs: None,
        }),
    )
    .await?;
    complete_wave4(&state, &receipt, &created.id).await?;
    Ok(Json(AssistantDeveloperAppResponse {
        resource: AssistantDeveloperAppResource {
            client_id: created.id,
        },
        replayed: false,
        client_secret: created.client_secret,
    }))
}

async fn update_developer_app_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantDeveloperAppUpdateRequest>,
) -> AppResult<Json<AssistantDeveloperAppResponse>> {
    let actor = auth_user.user_id.to_string();
    let id = parse_uuid(&body.client_id, "clientId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = action_fingerprint(
        DEVELOPER_APP_UPDATE_ACTION,
        serde_json::json!({"id":id,"name":body.name,"redirectUris":body.redirect_uris}),
    )?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        DEVELOPER_APP_UPDATE_ACTION,
        &request_id,
        &fp,
        id.clone(),
    )
    .await?;
    if let Some((_, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantDeveloperAppResponse {
            resource: AssistantDeveloperAppResource { client_id: id },
            replayed,
            client_secret: None,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let _ = developer_apps::update_my_oauth_client(
        State(state.clone()),
        auth_user,
        Path(id.clone()),
        Json(developer_apps::UpdateDeveloperOAuthClientRequest {
            name: body.name,
            redirect_uris: body.redirect_uris,
            delegation_scopes: None,
            broker_capability_enabled: None,
            revocation_webhook_url: None,
            revocation_webhook_secret: None,
            allowed_scopes: None,
            default_service_catalog_slugs: None,
        }),
    )
    .await?;
    complete_wave4(&state, &receipt, &id).await?;
    Ok(Json(AssistantDeveloperAppResponse {
        resource: AssistantDeveloperAppResource { client_id: id },
        replayed: false,
        client_secret: None,
    }))
}

async fn delete_developer_app_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantDeveloperAppRequest>,
) -> AppResult<Json<AssistantDeveloperAppResponse>> {
    let actor = auth_user.user_id.to_string();
    let id = parse_uuid(&body.client_id, "clientId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = action_fingerprint(DEVELOPER_APP_DELETE_ACTION, serde_json::json!({"id":id}))?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        DEVELOPER_APP_DELETE_ACTION,
        &request_id,
        &fp,
        id.clone(),
    )
    .await?;
    if let Some((_, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantDeveloperAppResponse {
            resource: AssistantDeveloperAppResource { client_id: id },
            replayed,
            client_secret: None,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let _ =
        developer_apps::delete_my_oauth_client(State(state.clone()), auth_user, Path(id.clone()))
            .await?;
    complete_wave4(&state, &receipt, &id).await?;
    Ok(Json(AssistantDeveloperAppResponse {
        resource: AssistantDeveloperAppResource { client_id: id },
        replayed: false,
        client_secret: None,
    }))
}

async fn rotate_developer_app_secret_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantDeveloperAppRequest>,
) -> AppResult<Json<AssistantDeveloperAppResponse>> {
    let actor = auth_user.user_id.to_string();
    let id = parse_uuid(&body.client_id, "clientId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = action_fingerprint(
        DEVELOPER_APP_ROTATE_SECRET_ACTION,
        serde_json::json!({"id":id}),
    )?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        DEVELOPER_APP_ROTATE_SECRET_ACTION,
        &request_id,
        &fp,
        id.clone(),
    )
    .await?;
    if let Some((_, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantDeveloperAppResponse {
            resource: AssistantDeveloperAppResource { client_id: id },
            replayed,
            client_secret: None,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let rotated = developer_apps::rotate_my_oauth_client_secret(
        State(state.clone()),
        auth_user,
        TelemetryContext::default(),
        Path(id.clone()),
    )
    .await?;
    let rotated_secret = rotated.0.client_secret;
    complete_wave4(&state, &receipt, &id).await?;
    Ok(Json(AssistantDeveloperAppResponse {
        resource: AssistantDeveloperAppResource { client_id: id },
        replayed: false,
        client_secret: Some(rotated_secret),
    }))
}

async fn update_notifications_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantNotificationsUpdateRequest>,
) -> AppResult<Json<AssistantNotificationResponse>> {
    let actor = auth_user.user_id.to_string();
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = action_fingerprint(
        NOTIFICATIONS_UPDATE_ACTION,
        serde_json::json!({"telegramEnabled":body.telegram_enabled,"approvalRequired":body.approval_required,"approvalTimeoutSecs":body.approval_timeout_secs,"grantExpiryDays":body.grant_expiry_days,"pushEnabled":body.push_enabled}),
    )?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        NOTIFICATIONS_UPDATE_ACTION,
        &request_id,
        &fp,
        actor.clone(),
    )
    .await?;
    if let Some((id, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantNotificationResponse {
            resource: AssistantNotificationBindingResource { binding_id: id },
            replayed,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let Json(settings) = notifications::update_settings(
        State(state.clone()),
        auth_user,
        Json(notifications::UpdateNotificationSettingsRequest {
            telegram_enabled: body.telegram_enabled,
            approval_required: body.approval_required,
            approval_timeout_secs: body.approval_timeout_secs,
            grant_expiry_days: body.grant_expiry_days,
            push_enabled: body.push_enabled,
        }),
    )
    .await?;
    let id = settings.id;
    complete_wave4(&state, &receipt, &id).await?;
    Ok(Json(AssistantNotificationResponse {
        resource: AssistantNotificationBindingResource { binding_id: id },
        replayed: false,
    }))
}

async fn link_telegram_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantActionRequestId>,
) -> AppResult<Json<AssistantTelegramLinkResponse>> {
    let actor = auth_user.user_id.to_string();
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = action_fingerprint(
        NOTIFICATIONS_TELEGRAM_LINK_ACTION,
        serde_json::json!({"userId":actor}),
    )?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        NOTIFICATIONS_TELEGRAM_LINK_ACTION,
        &request_id,
        &fp,
        actor.clone(),
    )
    .await?;
    if let Some((id, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantTelegramLinkResponse {
            resource: AssistantNotificationBindingResource { binding_id: id },
            replayed,
            link_code: None,
            bot_username: None,
            expires_in_secs: None,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let Json(link) = notifications::telegram_link(State(state.clone()), auth_user.clone()).await?;
    let Json(settings) = notifications::get_settings(State(state.clone()), auth_user).await?;
    complete_wave4(&state, &receipt, &settings.id).await?;
    Ok(Json(AssistantTelegramLinkResponse {
        resource: AssistantNotificationBindingResource {
            binding_id: settings.id,
        },
        replayed: false,
        link_code: Some(link.link_code),
        bot_username: Some(link.bot_username),
        expires_in_secs: Some(link.expires_in_secs),
    }))
}

async fn disconnect_telegram_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantActionRequestId>,
) -> AppResult<Json<AssistantNotificationResponse>> {
    let actor = auth_user.user_id.to_string();
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = action_fingerprint(
        NOTIFICATIONS_TELEGRAM_DISCONNECT_ACTION,
        serde_json::json!({"userId":actor}),
    )?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        NOTIFICATIONS_TELEGRAM_DISCONNECT_ACTION,
        &request_id,
        &fp,
        actor.clone(),
    )
    .await?;
    if let Some((id, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantNotificationResponse {
            resource: AssistantNotificationBindingResource { binding_id: id },
            replayed,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let _ = notifications::telegram_disconnect(
        State(state.clone()),
        auth_user.clone(),
        TelemetryContext::default(),
    )
    .await?;
    let Json(settings) = notifications::get_settings(State(state.clone()), auth_user).await?;
    complete_wave4(&state, &receipt, &settings.id).await?;
    Ok(Json(AssistantNotificationResponse {
        resource: AssistantNotificationBindingResource {
            binding_id: settings.id,
        },
        replayed: false,
    }))
}

async fn add_gcp_service_account_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantGcpCreateRequest>,
) -> AppResult<Json<AssistantExternalKeyResponse>> {
    let actor = auth_user.user_id.to_string();
    let fp = gcp_create_fingerprint(&body)?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        EXTERNAL_KEY_ADD_GCP_ACTION,
        &request_id,
        &fp,
        Uuid::new_v4().to_string(),
    )
    .await?;
    if let Some((id, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantExternalKeyResponse {
            resource: AssistantExternalKeyResource {
                external_key_id: id,
            },
            replayed,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let (_, Json(created)) = user_api_keys_external::create_gcp_service_account_key(
        State(state.clone()),
        auth_user,
        Json(user_api_keys_external::CreateGcpServiceAccountRequest {
            label: body.label,
            key_json: body.key_json,
            scopes: body.scopes,
            service_slugs: body.service_slugs.unwrap_or_default(),
            target_org_id: body.target_org_id,
        }),
    )
    .await?;
    complete_wave4(&state, &receipt, &created.id).await?;
    Ok(Json(AssistantExternalKeyResponse {
        resource: AssistantExternalKeyResource {
            external_key_id: created.id,
        },
        replayed: false,
    }))
}

async fn connect_openclaw_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantOpenClawConnectRequest>,
) -> AppResult<Json<AssistantUserServiceResponse>> {
    let actor = auth_user.user_id.to_string();
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let gateway_url = body.gateway_url.trim().to_string();
    let credential = body.credential;
    if gateway_url.is_empty() || credential.is_empty() {
        return Err(AppError::ValidationError(
            "gatewayUrl and credential are required".to_string(),
        ));
    }
    let parsed = url::Url::parse(&gateway_url)
        .map_err(|_| AppError::ValidationError("gatewayUrl must be a valid URL".to_string()))?;
    if parsed.scheme() != "https"
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(AppError::ValidationError(
            "gatewayUrl must be HTTPS without userinfo or a fragment".to_string(),
        ));
    }
    let fp = openclaw_connect_fingerprint(&gateway_url, &credential, body.label.as_deref())?;
    let outcome = reserve_or_replay(
        &state.db,
        &actor,
        OPENCLAW_CONNECT_ACTION,
        &request_id,
        &fp,
        Uuid::new_v4().to_string(),
    )
    .await?;
    if let Some((id, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantUserServiceResponse {
            resource: AssistantUserServiceResource {
                user_service_id: id,
            },
            replayed,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(_) => return Err(in_progress_conflict()),
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let Json(created) = keys::create_key(
        State(state.clone()),
        auth_user,
        TelemetryContext::default(),
        Json(keys::CreateKeyRequest {
            service_slug: Some("llm-openclaw".to_string()),
            credential: Some(credential),
            label: body.label.unwrap_or_else(|| "OpenClaw".to_string()),
            endpoint_url: Some(gateway_url),
            slug: None,
            auth_method: Some("bearer".to_string()),
            auth_key_name: Some("Authorization".to_string()),
            node_id: None,
            admin_only: None,
            ssh_host: None,
            ssh_port: None,
            ssh_certificate_auth: None,
            ssh_auth_mode: None,
            ssh_principals: None,
            ssh_certificate_ttl_minutes: None,
            identity_propagation_mode: None,
            identity_include_user_id: None,
            identity_include_email: None,
            identity_include_name: None,
            identity_jwt_audience: None,
            forward_access_token: None,
            inject_delegation_token: None,
            delegation_token_scope: None,
            target_org_id: None,
            openapi_spec_url: None,
            ws_frame_injections: None,
            oauth_client_id: None,
            oauth_client_secret: None,
            copy_oauth_client_from: None,
        }),
    )
    .await?;
    complete_wave4(&state, &receipt, &created.id).await?;
    Ok(Json(AssistantUserServiceResponse {
        resource: AssistantUserServiceResource {
            user_service_id: created.id,
        },
        replayed: false,
    }))
}

#[cfg(test)]
mod wave4_effect_tests {
    use super::*;

    /// `account.delete` is an irreversible cascade. The browser asks the user
    /// to type their email, but a browser-side comparison is not a control:
    /// the effect route is mounted and reachable, so anyone holding a
    /// first-party session could POST it directly.
    ///
    /// Falsifier: drop the `confirm_email` check in `delete_account` and this
    /// fails — the wrong-confirmation request returns 200 and the account is
    /// gone.
    #[tokio::test]
    async fn account_delete_rejects_missing_or_mismatched_confirmation() {
        use crate::models::user::{COLLECTION_NAME as USERS, UserType};
        use crate::test_utils::{connect_test_database, test_app_state, test_user};
        use axum::body::{Body, to_bytes};
        use axum::http::{Request, StatusCode};
        use serde_json::{Value, json};
        use tower::ServiceExt;

        let Some(db) = connect_test_database("assistant_account_delete_confirm").await else {
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        let mut actor = test_user(&actor_id, UserType::Person);
        actor.email = "owner@example.test".to_string();
        db.collection::<User>(USERS)
            .insert_one(&actor)
            .await
            .expect("insert actor");

        let state = test_app_state(db.clone());
        let token = crate::crypto::jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &Uuid::parse_str(&actor_id).expect("actor uuid"),
            "",
            None,
            None,
            None,
            None,
            None,
        )
        .expect("sign token");

        let post = |st: AppState, body: Value| {
            let token = token.clone();
            async move {
                let (_, private) = crate::routes::build_router();
                let response = private
                    .with_state(st)
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri("/api/v1/assistant/actions/org/account/delete")
                            .header("authorization", format!("Bearer {token}"))
                            .header("content-type", "application/json")
                            .body(Body::from(body.to_string()))
                            .expect("build request"),
                    )
                    .await
                    .expect("router responds");
                let status = response.status();
                let bytes = to_bytes(response.into_body(), 65_536).await.expect("body");
                (
                    status,
                    serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
                )
            }
        };

        // Wrong confirmation must be refused.
        let (status, _) = post(
            state.clone(),
            json!({ "actionRequestId": "delete-wrong", "confirmEmail": "someone@else.test" }),
        )
        .await;
        assert!(
            status.is_client_error(),
            "a mismatched confirmation must be refused, got {status}"
        );

        // Empty confirmation must be refused too.
        let (status, _) = post(
            state.clone(),
            json!({ "actionRequestId": "delete-empty", "confirmEmail": "   " }),
        )
        .await;
        assert!(
            status.is_client_error(),
            "an empty confirmation must be refused, got {status}"
        );

        // The account must still be there after both refusals.
        let survivor = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &actor_id })
            .await
            .expect("query actor");
        assert!(
            survivor.is_some(),
            "a refused confirmation must not delete the account"
        );
    }

    /// The confirmation is semantic content, so reusing one `actionRequestId`
    /// with a different confirmation must conflict rather than replay.
    ///
    /// Falsifier: fingerprint without `confirm_email` and the two values below
    /// collide.
    #[test]
    fn account_delete_fingerprint_binds_the_confirmation() {
        let first = fingerprint_canonical(&ConfirmedAccountDeleteFingerprint {
            action: ACCOUNT_DELETE_ACTION,
            user_id: "user-1",
            confirm_email: "owner@example.test",
        })
        .unwrap();
        let second = fingerprint_canonical(&ConfirmedAccountDeleteFingerprint {
            action: ACCOUNT_DELETE_ACTION,
            user_id: "user-1",
            confirm_email: "someone@else.test",
        })
        .unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn fingerprints_include_semantic_request_content() {
        let first = action_fingerprint(
            ORG_UPDATE_ACTION,
            serde_json::json!({ "orgId": "00000000-0000-0000-0000-000000000001", "displayName": "one" }),
        )
        .unwrap();
        let second = action_fingerprint(
            ORG_UPDATE_ACTION,
            serde_json::json!({ "orgId": "00000000-0000-0000-0000-000000000001", "displayName": "two" }),
        )
        .unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn sensitive_request_debug_is_redacted() {
        let request = AssistantOpenClawConnectRequest {
            action_request_id: "request-1".to_string(),
            gateway_url: "https://gateway.example".to_string(),
            credential: "Bearer nyxid_ag_abcdefghijklmnop".to_string(),
            label: Some("OpenClaw".to_string()),
        };
        let debug = format!("{request:?}");
        assert!(!debug.contains("nyxid_ag_abcdefghijklmnop"));
        assert!(debug.contains("REDACTED"));
    }

    #[test]
    fn sensitive_and_browser_collected_inputs_change_fingerprints() {
        let base = service_account_create_fingerprint("deploy", None, Some("proxy")).unwrap();
        let widened =
            service_account_create_fingerprint("deploy", None, Some("proxy admin")).unwrap();
        assert_ne!(base, widened);

        let gcp = AssistantGcpCreateRequest {
            action_request_id: "request-gcp".to_string(),
            label: Some("gcp".to_string()),
            key_json: "{\"private_key\":\"one\"}".to_string(),
            scopes: None,
            service_slugs: None,
            target_org_id: None,
        };
        let gcp_one = gcp_create_fingerprint(&gcp).unwrap();
        let gcp_two = gcp_create_fingerprint(&AssistantGcpCreateRequest {
            key_json: "{\"private_key\":\"two\"}".to_string(),
            ..gcp
        })
        .unwrap();
        assert_ne!(gcp_one, gcp_two);

        let openclaw_one =
            openclaw_connect_fingerprint("https://gateway.example", "bearer-one", None).unwrap();
        let openclaw_two =
            openclaw_connect_fingerprint("https://gateway.example", "bearer-two", None).unwrap();
        assert_ne!(openclaw_one, openclaw_two);
    }

    #[test]
    fn sensitive_descriptor_params_are_rejected() {
        let service_account =
            serde_json::from_value::<AssistantServiceAccountCreateRequest>(serde_json::json!({
                "actionRequestId": "request-1",
                "name": "deploy",
                "credential": "Bearer forbidden",
            }));
        assert!(service_account.is_err());

        let openclaw =
            serde_json::from_value::<AssistantOpenClawConnectRequest>(serde_json::json!({
                "actionRequestId": "request-2",
                "gatewayUrl": "https://gateway.example",
                "credential": "browser-only",
                "clientSecret": "chat-supplied",
            }));
        assert!(openclaw.is_err());
    }
}
