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
use mongodb::bson::{Document, doc};
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
    OneTimeMaterialAvailability, ReceiptOutcome, fingerprint_canonical, in_progress_conflict,
    mark_completed, normalize_action_request_id, reserve_or_replay,
};
use crate::services::{approval_service, org_service, service_account_service};
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
    pub confirmed: bool,
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
    #[serde(default)]
    pub one_time_material: OneTimeMaterialAvailability,
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
    #[serde(default)]
    pub one_time_material: OneTimeMaterialAvailability,
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
    #[serde(default)]
    pub one_time_material: OneTimeMaterialAvailability,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
}
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantDeveloperAppResponse {
    pub resource: AssistantDeveloperAppResource,
    pub replayed: bool,
    #[serde(default)]
    pub one_time_material: OneTimeMaterialAvailability,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
}
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantExternalKeyResponse {
    pub resource: AssistantExternalKeyResource,
    pub replayed: bool,
    #[serde(default)]
    pub one_time_material: OneTimeMaterialAvailability,
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
    #[serde(default)]
    pub one_time_material: OneTimeMaterialAvailability,
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
pub struct AssistantConfirmedOrgIdRequest {
    pub action_request_id: String,
    pub org_id: String,
    pub confirmed: bool,
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
pub struct AssistantConfirmedOrgMemberRequest {
    pub action_request_id: String,
    pub org_id: String,
    pub member_id: String,
    pub confirmed: bool,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantOrgMemberRoleRequest {
    pub action_request_id: String,
    pub org_id: String,
    pub member_id: String,
    pub role: orgs::OrgRoleWire,
    pub expected_role: orgs::OrgRoleWire,
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
    pub target_org_id: Option<String>,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantServiceAccountRotateRequest {
    pub action_request_id: String,
    pub service_account_id: String,
    pub expected_updated_at: String,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantConfirmedServiceAccountRequest {
    pub action_request_id: String,
    pub service_account_id: String,
    pub confirmed: bool,
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
pub struct AssistantDeveloperAppRotateRequest {
    pub action_request_id: String,
    pub client_id: String,
    pub expected_updated_at: String,
}
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantConfirmedDeveloperAppRequest {
    pub action_request_id: String,
    pub client_id: String,
    pub confirmed: bool,
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
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantConfirmedActionRequestId {
    pub action_request_id: String,
    pub confirmed: bool,
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
    confirmed: bool,
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

fn require_destructive_confirmation(confirmed: bool) -> AppResult<()> {
    if !confirmed {
        return Err(AppError::ValidationError(
            "confirmed must be true for this destructive assistant action".to_string(),
        ));
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
    require_destructive_confirmation(body.confirmed)?;
    let client_id = parse_uuid(&body.client_id, "clientId")?;
    let user_id = auth_user.user_id.to_string();
    require_user(&state, &user_id).await?;
    let exists = consent_exists(&state, &user_id, &client_id).await?;
    let fingerprint = fingerprint_canonical(&ConsentFingerprint {
        action: ACCOUNT_REVOKE_CONSENT_ACTION,
        user_id: &user_id,
        client_id: &client_id,
        confirmed: body.confirmed,
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
        ReceiptOutcome::InProgress(receipt) => {
            if exists {
                return Err(in_progress_conflict());
            }
            mark_completed(&state.db, &receipt).await?;
            true
        }
        ReceiptOutcome::Reserved(receipt) => {
            if !exists {
                discard_pending_wave4(&state, &receipt).await?;
                return Err(AppError::ConsentNotFound);
            }
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
                ReceiptOutcome::Replay(receipt) => {
                    let factor = state
                        .db
                        .collection::<MfaFactor>(MFA_FACTORS)
                        .find_one(doc! {
                            "user_id": &user_id,
                            "factor_type": "totp",
                            "created_at": { "$gte": bson::DateTime::from_chrono(receipt.created_at) },
                        })
                        .await?;
                    return Ok(Json(AssistantMfaSetupResponse {
                        resource: AssistantAccountResource { user_id },
                        stage: AssistantMfaSetupStage::Start,
                        factor_id: factor.map(|factor| factor.id),
                        setup_value: None,
                        qr_code_url: None,
                        recovery_values: None,
                        replayed: true,
                        one_time_material: OneTimeMaterialAvailability::Unavailable,
                    }));
                }
                ReceiptOutcome::InProgress(receipt) => {
                    let factor = state
                        .db
                        .collection::<MfaFactor>(MFA_FACTORS)
                        .find_one(doc! {
                            "user_id": &user_id,
                            "factor_type": "totp",
                            "created_at": { "$gte": bson::DateTime::from_chrono(receipt.created_at) },
                        })
                        .await?;
                    let Some(factor) = factor else {
                        return Err(in_progress_conflict());
                    };
                    mark_completed(&state.db, &receipt).await?;
                    return Ok(Json(AssistantMfaSetupResponse {
                        resource: AssistantAccountResource { user_id },
                        stage: AssistantMfaSetupStage::Start,
                        factor_id: Some(factor.id),
                        setup_value: None,
                        qr_code_url: None,
                        recovery_values: None,
                        replayed: true,
                        one_time_material: OneTimeMaterialAvailability::Unavailable,
                    }));
                }
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
                one_time_material: OneTimeMaterialAvailability::Delivered,
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
                    one_time_material: OneTimeMaterialAvailability::Unavailable,
                })),
                ReceiptOutcome::InProgress(receipt) => {
                    let user = require_user(&state, &user_id).await?;
                    let pinned_factor_verified = state
                        .db
                        .collection::<MfaFactor>(MFA_FACTORS)
                        .find_one(doc! {
                            "_id": &factor_id,
                            "user_id": &user_id,
                            "factor_type": "totp",
                            "is_active": true,
                            "is_verified": true,
                        })
                        .await?
                        .is_some();
                    if !user.mfa_enabled || !pinned_factor_verified {
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
                        one_time_material: OneTimeMaterialAvailability::Unavailable,
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
                        one_time_material: OneTimeMaterialAvailability::Delivered,
                    }))
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
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

#[derive(Serialize)]
#[serde(tag = "action", content = "request", rename_all_fields = "camelCase")]
enum Wave4Fingerprint<'a> {
    #[serde(rename = "org.create")]
    OrgCreate {
        display_name: &'a str,
        contact_email: Option<&'a str>,
        avatar_url: Option<&'a str>,
    },
    #[serde(rename = "org.update")]
    OrgUpdate {
        org_id: &'a str,
        display_name: Option<&'a str>,
        slug: Option<&'a str>,
        contact_email: Option<&'a str>,
        avatar_url: Option<&'a str>,
    },
    #[serde(rename = "org.delete")]
    OrgDelete { org_id: &'a str, confirmed: bool },
    #[serde(rename = "org.member_add")]
    OrgMemberAdd {
        org_id: &'a str,
        user_id: &'a str,
        role: orgs::OrgRoleWire,
        allowed_service_ids: Option<&'a [String]>,
    },
    #[serde(rename = "org.member_remove")]
    OrgMemberRemove {
        org_id: &'a str,
        member_id: &'a str,
        confirmed: bool,
    },
    #[serde(rename = "org.member_update_role")]
    OrgMemberUpdateRole {
        org_id: &'a str,
        member_id: &'a str,
        role: orgs::OrgRoleWire,
        expected_role: orgs::OrgRoleWire,
    },
    #[serde(rename = "org.invite")]
    OrgInvite {
        org_id: &'a str,
        role: orgs::OrgRoleWire,
        allowed_service_ids: Option<&'a [String]>,
        ttl_hours: Option<i64>,
    },
    #[serde(rename = "org.set_primary")]
    OrgSetPrimary { org_id: &'a str },
    #[serde(rename = "service_account.create")]
    ServiceAccountCreate {
        name: &'a str,
        description: Option<&'a str>,
        allowed_scopes: Option<&'a str>,
        target_org_id: Option<&'a str>,
    },
    #[serde(rename = "service_account.update")]
    ServiceAccountUpdate {
        #[serde(rename = "id")]
        service_account_id: &'a str,
        name: Option<&'a str>,
        description: Option<&'a str>,
    },
    #[serde(rename = "service_account.delete")]
    ServiceAccountDelete {
        #[serde(rename = "id")]
        service_account_id: &'a str,
        confirmed: bool,
    },
    #[serde(rename = "service_account.rotate_secret")]
    ServiceAccountRotate {
        #[serde(rename = "id")]
        service_account_id: &'a str,
        expected_updated_at: &'a str,
    },
    #[serde(rename = "service_account.revoke_tokens")]
    ServiceAccountRevokeTokens {
        #[serde(rename = "id")]
        service_account_id: &'a str,
        confirmed: bool,
    },
    #[serde(rename = "developer_app.create")]
    DeveloperAppCreate {
        name: &'a str,
        redirect_uris: &'a [String],
    },
    #[serde(rename = "developer_app.update")]
    DeveloperAppUpdate {
        #[serde(rename = "id")]
        client_id: &'a str,
        name: Option<&'a str>,
        redirect_uris: Option<&'a [String]>,
    },
    #[serde(rename = "developer_app.delete")]
    DeveloperAppDelete {
        #[serde(rename = "id")]
        client_id: &'a str,
        confirmed: bool,
    },
    #[serde(rename = "developer_app.rotate_secret")]
    DeveloperAppRotate {
        #[serde(rename = "id")]
        client_id: &'a str,
        expected_updated_at: &'a str,
    },
    #[serde(rename = "notifications.update")]
    NotificationsUpdate {
        telegram_enabled: Option<bool>,
        approval_required: Option<bool>,
        approval_timeout_secs: Option<u32>,
        grant_expiry_days: Option<u32>,
        push_enabled: Option<bool>,
    },
    #[serde(rename = "notifications.telegram_link")]
    NotificationsTelegramLink { user_id: &'a str },
    #[serde(rename = "notifications.telegram_disconnect")]
    NotificationsTelegramDisconnect { user_id: &'a str, confirmed: bool },
    #[serde(rename = "external_key.add_gcp_service_account")]
    ExternalKeyAddGcp {
        label: Option<&'a str>,
        #[serde(rename = "keyJson")]
        key_json_fingerprint: &'a str,
        scopes: Option<&'a str>,
        service_slugs: Option<&'a [String]>,
        target_org_id: Option<&'a str>,
    },
    #[serde(rename = "openclaw.connect")]
    OpenClawConnect {
        gateway_url: &'a str,
        #[serde(rename = "credential")]
        credential_fingerprint: &'a str,
        label: Option<&'a str>,
    },
}

fn wave4_fingerprint(value: &Wave4Fingerprint<'_>) -> AppResult<String> {
    fingerprint_canonical(value)
}

fn service_account_create_fingerprint(
    name: &str,
    description: Option<&str>,
    allowed_scopes: Option<&str>,
    target_org_id: Option<&str>,
) -> AppResult<String> {
    wave4_fingerprint(&Wave4Fingerprint::ServiceAccountCreate {
        name,
        description,
        allowed_scopes,
        target_org_id,
    })
}

fn gcp_create_fingerprint(body: &AssistantGcpCreateRequest) -> AppResult<String> {
    let key_json_fingerprint =
        crate::services::assistant_action_receipts::fingerprint_sensitive_material(&body.key_json);
    wave4_fingerprint(&Wave4Fingerprint::ExternalKeyAddGcp {
        label: body.label.as_deref(),
        key_json_fingerprint: &key_json_fingerprint,
        scopes: body.scopes.as_deref(),
        service_slugs: body.service_slugs.as_deref(),
        target_org_id: body.target_org_id.as_deref(),
    })
}

fn openclaw_connect_fingerprint(
    gateway_url: &str,
    credential: &str,
    label: Option<&str>,
) -> AppResult<String> {
    let credential_fingerprint =
        crate::services::assistant_action_receipts::fingerprint_sensitive_material(credential);
    wave4_fingerprint(&Wave4Fingerprint::OpenClawConnect {
        gateway_url,
        credential_fingerprint: &credential_fingerprint,
        label,
    })
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

async fn discard_pending_wave4(
    state: &AppState,
    receipt: &crate::models::assistant_action_receipt::AssistantActionReceipt,
) -> AppResult<()> {
    state
        .db
        .collection::<crate::models::assistant_action_receipt::AssistantActionReceipt>(
            crate::models::assistant_action_receipt::COLLECTION_NAME,
        )
        .delete_one(doc! { "_id": &receipt.id, "status": "pending" })
        .await?;
    Ok(())
}

fn updated_since_receipt(
    updated_at: chrono::DateTime<chrono::Utc>,
    receipt: &crate::models::assistant_action_receipt::AssistantActionReceipt,
) -> bool {
    // MongoDB stores millisecond precision, while a freshly reserved receipt
    // still carries Chrono's finer in-memory precision.
    updated_at.timestamp_millis() >= receipt.created_at.timestamp_millis()
}

fn parse_browser_updated_at(value: &str, field: &str) -> AppResult<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&chrono::Utc))
        .map_err(|_| AppError::ValidationError(format!("{field} must be an RFC 3339 timestamp")))
}

fn response_updated_since_receipt(
    value: &str,
    receipt: &crate::models::assistant_action_receipt::AssistantActionReceipt,
) -> AppResult<bool> {
    let updated_at = chrono::DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&chrono::Utc))
        .map_err(|error| AppError::Internal(format!("invalid response updated_at: {error}")))?;
    Ok(updated_since_receipt(updated_at, receipt))
}

async fn require_assistant_org_access(
    state: &AppState,
    actor: &str,
    org_id: &str,
    require_admin: bool,
) -> AppResult<()> {
    let access = org_service::resolve_owner_access(&state.db, actor, org_id).await?;
    if !access.can_read() {
        return Err(AppError::NotFound("Organization not found".to_string()));
    }
    if require_admin && !access.can_write() {
        return Err(AppError::OrgRoleInsufficient(
            "admin role required for this operation".to_string(),
        ));
    }
    Ok(())
}

fn replayed_resource(outcome: &ReceiptOutcome) -> Option<(String, bool)> {
    match outcome {
        ReceiptOutcome::Replay(receipt) => Some((receipt.resource_id.clone(), true)),
        _ => None,
    }
}

async fn receipt_resource_exists(
    state: &AppState,
    collection: &str,
    receipt: &crate::models::assistant_action_receipt::AssistantActionReceipt,
) -> AppResult<bool> {
    Ok(state
        .db
        .collection::<Document>(collection)
        .find_one(doc! { "_id": &receipt.resource_id })
        .await?
        .is_some())
}

async fn create_org_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantOrgCreateRequest>,
) -> AppResult<Json<AssistantOrgResponse>> {
    let actor = auth_user.user_id.to_string();
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = wave4_fingerprint(&Wave4Fingerprint::OrgCreate {
        display_name: &body.display_name,
        contact_email: body.contact_email.as_deref(),
        avatar_url: body.avatar_url.as_deref(),
    })?;
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
        ReceiptOutcome::InProgress(receipt) => {
            if !receipt_resource_exists(&state, USERS, &receipt).await? {
                return Err(in_progress_conflict());
            }
            complete_wave4(&state, &receipt, &receipt.resource_id).await?;
            return Ok(Json(AssistantOrgResponse {
                resource: AssistantOrgResource {
                    org_id: receipt.resource_id,
                },
                replayed: true,
            }));
        }
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let (_, Json(org)) = orgs::create_org_with_id(
        State(state.clone()),
        auth_user,
        Json(orgs::CreateOrgRequest {
            display_name: body.display_name,
            contact_email: body.contact_email,
            avatar_url: body.avatar_url,
        }),
        Some(&receipt.resource_id),
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
    require_assistant_org_access(&state, &actor, &org_id, true).await?;
    let fp = wave4_fingerprint(&Wave4Fingerprint::OrgUpdate {
        org_id: &org_id,
        display_name: body.display_name.as_deref(),
        slug: body.slug.as_deref(),
        contact_email: body.contact_email.as_deref(),
        avatar_url: body.avatar_url.as_deref(),
    })?;
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
        ReceiptOutcome::InProgress(receipt) => {
            let current = require_user(&state, &org_id).await?;
            let display_name_matches = body
                .display_name
                .as_ref()
                .is_none_or(|value| current.display_name.as_ref() == Some(value));
            let slug_matches = body
                .slug
                .as_ref()
                .is_none_or(|value| current.slug.as_ref() == Some(value));
            let avatar_matches = body
                .avatar_url
                .as_ref()
                .is_none_or(|value| current.avatar_url.as_ref() == Some(value));
            let contact_matches = body.contact_email.as_ref().is_none_or(|value| {
                let value = value.trim();
                if value.is_empty() {
                    org_service::contact_email_for_display(&current).is_none()
                } else {
                    current.email == value
                }
            });
            if !display_name_matches
                || !slug_matches
                || !avatar_matches
                || !contact_matches
                || !updated_since_receipt(current.updated_at, &receipt)
            {
                return Err(in_progress_conflict());
            }
            complete_wave4(&state, &receipt, &org_id).await?;
            return Ok(Json(AssistantOrgResponse {
                resource: AssistantOrgResource { org_id },
                replayed: true,
            }));
        }
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
    Json(body): Json<AssistantConfirmedOrgIdRequest>,
) -> AppResult<Json<AssistantOrgResponse>> {
    require_destructive_confirmation(body.confirmed)?;
    let actor = auth_user.user_id.to_string();
    let org_id = parse_uuid(&body.org_id, "orgId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = wave4_fingerprint(&Wave4Fingerprint::OrgDelete {
        org_id: &org_id,
        confirmed: body.confirmed,
    })?;
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
        ReceiptOutcome::Reserved(receipt) => {
            if let Err(error) = require_assistant_org_access(&state, &actor, &org_id, true).await {
                discard_pending_wave4(&state, &receipt).await?;
                return Err(error);
            }
            receipt
        }
        ReceiptOutcome::InProgress(receipt) => {
            let exists = state
                .db
                .collection::<User>(USERS)
                .find_one(doc! { "_id": &org_id })
                .await?
                .is_some();
            if exists {
                require_assistant_org_access(&state, &actor, &org_id, true).await?;
                return Err(in_progress_conflict());
            }
            complete_wave4(&state, &receipt, &org_id).await?;
            return Ok(Json(AssistantOrgResponse {
                resource: AssistantOrgResource { org_id },
                replayed: true,
            }));
        }
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
    require_assistant_org_access(&state, &actor, &org_id, true).await?;
    let fp = wave4_fingerprint(&Wave4Fingerprint::OrgMemberAdd {
        org_id: &org_id,
        user_id: &user_id,
        role: body.role,
        allowed_service_ids: body.allowed_service_ids.as_deref(),
    })?;
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
        ReceiptOutcome::InProgress(receipt) => {
            let current = org_service::get_active_membership(&state.db, &org_id, &user_id).await?;
            let already_applied = current.is_some_and(|membership| {
                membership.role == body.role.into()
                    && membership.allowed_service_ids == body.allowed_service_ids
                    && updated_since_receipt(membership.created_at, &receipt)
            });
            if !already_applied {
                return Err(in_progress_conflict());
            }
            complete_wave4(&state, &receipt, &org_id).await?;
            return Ok(Json(AssistantOrgResponse {
                resource: AssistantOrgResource { org_id },
                replayed: true,
            }));
        }
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
    Json(body): Json<AssistantConfirmedOrgMemberRequest>,
) -> AppResult<Json<AssistantOrgResponse>> {
    require_destructive_confirmation(body.confirmed)?;
    let actor = auth_user.user_id.to_string();
    let org_id = parse_uuid(&body.org_id, "orgId")?;
    let member_id = parse_uuid(&body.member_id, "memberId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    require_assistant_org_access(&state, &actor, &org_id, true).await?;
    let fp = wave4_fingerprint(&Wave4Fingerprint::OrgMemberRemove {
        org_id: &org_id,
        member_id: &member_id,
        confirmed: body.confirmed,
    })?;
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
        ReceiptOutcome::InProgress(receipt) => {
            let revoked = state
                .db
                .collection::<crate::models::org_membership::OrgMembership>(
                    crate::models::org_membership::COLLECTION_NAME,
                )
                .find_one(doc! {
                    "org_user_id": &org_id,
                    "member_user_id": &member_id,
                    "revoked_at": {
                        "$gte": bson::DateTime::from_millis(receipt.created_at.timestamp_millis()),
                    },
                })
                .await?
                .is_some();
            if !revoked {
                return Err(in_progress_conflict());
            }
            complete_wave4(&state, &receipt, &org_id).await?;
            return Ok(Json(AssistantOrgResponse {
                resource: AssistantOrgResource { org_id },
                replayed: true,
            }));
        }
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
    require_assistant_org_access(&state, &actor, &org_id, true).await?;
    let update = orgs::UpdateMemberRequest {
        role: Some(body.role),
        scope_source: None,
        allowed_service_ids: None,
    };
    let current =
        orgs::authorize_member_update(&state, &actor, &org_id, &member_id, &update).await?;
    let fp = wave4_fingerprint(&Wave4Fingerprint::OrgMemberUpdateRole {
        org_id: &org_id,
        member_id: &member_id,
        role: body.role,
        expected_role: body.expected_role,
    })?;
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
        ReceiptOutcome::Reserved(receipt) => {
            if orgs::OrgRoleWire::from(current.role) != body.expected_role {
                discard_pending_wave4(&state, &receipt).await?;
                return Err(AppError::Conflict(
                    "organization member role changed before confirmation".to_string(),
                ));
            }
            receipt
        }
        ReceiptOutcome::InProgress(receipt) => {
            let current =
                org_service::get_active_membership(&state.db, &org_id, &member_id).await?;
            if current.is_none_or(|membership| membership.role != body.role.into()) {
                return Err(in_progress_conflict());
            }
            complete_wave4(&state, &receipt, &org_id).await?;
            return Ok(Json(AssistantOrgResponse {
                resource: AssistantOrgResource { org_id },
                replayed: true,
            }));
        }
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let _ = orgs::update_member(
        State(state.clone()),
        auth_user,
        Path((org_id.clone(), member_id)),
        Json(update),
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
    require_assistant_org_access(&state, &actor, &org_id, true).await?;
    let fp = wave4_fingerprint(&Wave4Fingerprint::OrgInvite {
        org_id: &org_id,
        role: body.role,
        allowed_service_ids: body.allowed_service_ids.as_deref(),
        ttl_hours: body.ttl_hours,
    })?;
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
        ReceiptOutcome::InProgress(receipt) => {
            let scope_source = if body.allowed_service_ids.is_some() {
                "override"
            } else {
                "inherit"
            };
            let invite_exists = state
                .db
                .collection::<mongodb::bson::Document>(crate::models::org_invite::COLLECTION_NAME)
                .find_one(doc! {
                    "org_user_id": &org_id,
                    "created_by": &actor,
                    "role": crate::models::org_membership::OrgRole::from(body.role).as_str(),
                    "scope_source": scope_source,
                    "allowed_service_ids": bson::to_bson(&body.allowed_service_ids)
                        .map_err(|error| AppError::Internal(error.to_string()))?,
                    "created_at": { "$gte": bson::DateTime::from_chrono(receipt.created_at) },
                })
                .await?
                .is_some();
            if !invite_exists {
                return Err(in_progress_conflict());
            }
            complete_wave4(&state, &receipt, &org_id).await?;
            return Ok(Json(AssistantOrgResponse {
                resource: AssistantOrgResource { org_id },
                replayed: true,
            }));
        }
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
    require_assistant_org_access(&state, &actor, &org_id, false).await?;
    let fp = wave4_fingerprint(&Wave4Fingerprint::OrgSetPrimary { org_id: &org_id })?;
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
        ReceiptOutcome::InProgress(receipt) => {
            let current = require_user(&state, &actor).await?;
            if current.primary_org_id.as_deref() != Some(org_id.as_str())
                || !updated_since_receipt(current.updated_at, &receipt)
            {
                return Err(in_progress_conflict());
            }
            complete_wave4(&state, &receipt, &org_id).await?;
            return Ok(Json(AssistantOrgResponse {
                resource: AssistantOrgResource { org_id },
                replayed: true,
            }));
        }
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
    admin_service_accounts::resolve_service_account_create_owner(
        &state,
        &auth_user,
        body.target_org_id.as_deref(),
    )
    .await?;
    let fp = service_account_create_fingerprint(
        &body.name,
        body.description.as_deref(),
        body.allowed_scopes.as_deref(),
        body.target_org_id.as_deref(),
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
            one_time_material: OneTimeMaterialAvailability::Unavailable,
            client_secret: None,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(receipt) => {
            if !receipt_resource_exists(
                &state,
                crate::models::service_account::COLLECTION_NAME,
                &receipt,
            )
            .await?
            {
                return Err(in_progress_conflict());
            }
            mark_completed(&state.db, &receipt).await?;
            return Ok(Json(AssistantServiceAccountResponse {
                resource: AssistantServiceAccountResource {
                    service_account_id: receipt.resource_id,
                },
                replayed: true,
                one_time_material: OneTimeMaterialAvailability::Unavailable,
                client_secret: None,
            }));
        }
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let Json(created) = admin_service_accounts::create_service_account_with_id(
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
            target_org_id: body.target_org_id,
        }),
        Some(&receipt.resource_id),
    )
    .await?;
    complete_wave4(&state, &receipt, &created.id).await?;
    Ok(Json(AssistantServiceAccountResponse {
        resource: AssistantServiceAccountResource {
            service_account_id: created.id,
        },
        replayed: false,
        one_time_material: OneTimeMaterialAvailability::Delivered,
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
    let existing = service_account_service::get_service_account(&state.db, &id).await?;
    admin_service_accounts::require_admin_or_owning_org_admin(&state, &auth_user, &existing)
        .await?;
    let fp = wave4_fingerprint(&Wave4Fingerprint::ServiceAccountUpdate {
        service_account_id: &id,
        name: body.name.as_deref(),
        description: body.description.as_deref(),
    })?;
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
            one_time_material: OneTimeMaterialAvailability::Delivered,
            client_secret: None,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(receipt) => {
            let name_matches = body.name.as_ref().is_none_or(|name| existing.name == *name);
            let description_matches = body.description.as_ref().is_none_or(|description| {
                if description.is_empty() {
                    existing.description.is_none()
                } else {
                    existing.description.as_ref() == Some(description)
                }
            });
            if !name_matches
                || !description_matches
                || !updated_since_receipt(existing.updated_at, &receipt)
            {
                return Err(in_progress_conflict());
            }
            complete_wave4(&state, &receipt, &id).await?;
            return Ok(Json(AssistantServiceAccountResponse {
                resource: AssistantServiceAccountResource {
                    service_account_id: id,
                },
                replayed: true,
                one_time_material: OneTimeMaterialAvailability::Delivered,
                client_secret: None,
            }));
        }
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
        one_time_material: OneTimeMaterialAvailability::Delivered,
        client_secret: None,
    }))
}

async fn delete_service_account_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantConfirmedServiceAccountRequest>,
) -> AppResult<Json<AssistantServiceAccountResponse>> {
    require_destructive_confirmation(body.confirmed)?;
    let actor = auth_user.user_id.to_string();
    let id = parse_uuid(&body.service_account_id, "serviceAccountId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let existing = service_account_service::get_service_account(&state.db, &id).await?;
    admin_service_accounts::require_admin_or_owning_org_admin(&state, &auth_user, &existing)
        .await?;
    let fp = wave4_fingerprint(&Wave4Fingerprint::ServiceAccountDelete {
        service_account_id: &id,
        confirmed: body.confirmed,
    })?;
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
            one_time_material: OneTimeMaterialAvailability::Delivered,
            client_secret: None,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(receipt) => {
            if existing.is_active || !updated_since_receipt(existing.updated_at, &receipt) {
                return Err(in_progress_conflict());
            }
            complete_wave4(&state, &receipt, &id).await?;
            return Ok(Json(AssistantServiceAccountResponse {
                resource: AssistantServiceAccountResource {
                    service_account_id: id,
                },
                replayed: true,
                one_time_material: OneTimeMaterialAvailability::Delivered,
                client_secret: None,
            }));
        }
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
        one_time_material: OneTimeMaterialAvailability::Delivered,
        client_secret: None,
    }))
}

async fn rotate_service_account_secret_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantServiceAccountRotateRequest>,
) -> AppResult<Json<AssistantServiceAccountResponse>> {
    let actor = auth_user.user_id.to_string();
    let id = parse_uuid(&body.service_account_id, "serviceAccountId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let existing = service_account_service::get_service_account(&state.db, &id).await?;
    admin_service_accounts::require_admin_or_owning_org_admin(&state, &auth_user, &existing)
        .await?;
    let expected_updated_at =
        parse_browser_updated_at(&body.expected_updated_at, "expectedUpdatedAt")?;
    let expected_updated_at_fingerprint =
        expected_updated_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let secret_fingerprint =
        crate::services::assistant_action_receipts::fingerprint_sensitive_material(
            &existing.client_secret_hash,
        );
    let fp = wave4_fingerprint(&Wave4Fingerprint::ServiceAccountRotate {
        service_account_id: &id,
        expected_updated_at: &expected_updated_at_fingerprint,
    })?;
    let outcome = crate::services::assistant_action_receipts::reserve_or_replay_with_secret_marker(
        &state.db,
        &actor,
        SERVICE_ACCOUNT_ROTATE_SECRET_ACTION,
        &request_id,
        &fp,
        id.clone(),
        Some(secret_fingerprint),
    )
    .await?;
    if let Some((_, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantServiceAccountResponse {
            resource: AssistantServiceAccountResource {
                service_account_id: id,
            },
            replayed,
            one_time_material: OneTimeMaterialAvailability::Unavailable,
            client_secret: None,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(receipt) => {
            if existing.updated_at.timestamp_millis() != expected_updated_at.timestamp_millis() {
                discard_pending_wave4(&state, &receipt).await?;
                return Err(AppError::Conflict(
                    "service account changed before secret rotation".to_string(),
                ));
            }
            receipt
        }
        ReceiptOutcome::InProgress(receipt) => {
            let current = service_account_service::get_service_account(&state.db, &id).await?;
            let committed = receipt
                .resource_secret_fingerprint
                .as_ref()
                .is_some_and(|marker| {
                    crate::services::assistant_action_receipts::fingerprint_sensitive_material(
                        &current.client_secret_hash,
                    ) != *marker
                });
            if !committed {
                return Err(in_progress_conflict());
            }
            mark_completed(&state.db, &receipt).await?;
            return Ok(Json(AssistantServiceAccountResponse {
                resource: AssistantServiceAccountResource {
                    service_account_id: id,
                },
                replayed: true,
                one_time_material: OneTimeMaterialAvailability::Unavailable,
                client_secret: None,
            }));
        }
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
        one_time_material: OneTimeMaterialAvailability::Delivered,
        client_secret: Some(rotated_secret),
    }))
}

async fn revoke_service_account_tokens_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantConfirmedServiceAccountRequest>,
) -> AppResult<Json<AssistantServiceAccountResponse>> {
    require_destructive_confirmation(body.confirmed)?;
    let actor = auth_user.user_id.to_string();
    let id = parse_uuid(&body.service_account_id, "serviceAccountId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let existing = service_account_service::get_service_account(&state.db, &id).await?;
    admin_service_accounts::require_admin_or_owning_org_admin(&state, &auth_user, &existing)
        .await?;
    let fp = wave4_fingerprint(&Wave4Fingerprint::ServiceAccountRevokeTokens {
        service_account_id: &id,
        confirmed: body.confirmed,
    })?;
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
            one_time_material: OneTimeMaterialAvailability::Delivered,
            client_secret: None,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(receipt) => {
            let active_tokens = state
                .db
                .collection::<mongodb::bson::Document>(
                    crate::models::service_account_token::COLLECTION_NAME,
                )
                .count_documents(doc! { "service_account_id": &id, "revoked": false })
                .await?;
            if active_tokens != 0 || !updated_since_receipt(existing.updated_at, &receipt) {
                return Err(in_progress_conflict());
            }
            complete_wave4(&state, &receipt, &id).await?;
            return Ok(Json(AssistantServiceAccountResponse {
                resource: AssistantServiceAccountResource {
                    service_account_id: id,
                },
                replayed: true,
                one_time_material: OneTimeMaterialAvailability::Delivered,
                client_secret: None,
            }));
        }
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
        one_time_material: OneTimeMaterialAvailability::Delivered,
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
    let fp = wave4_fingerprint(&Wave4Fingerprint::DeveloperAppCreate {
        name: &body.name,
        redirect_uris: &body.redirect_uris,
    })?;
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
            one_time_material: OneTimeMaterialAvailability::Unavailable,
            client_secret: None,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(receipt) => {
            if !receipt_resource_exists(
                &state,
                crate::models::oauth_client::COLLECTION_NAME,
                &receipt,
            )
            .await?
            {
                return Err(in_progress_conflict());
            }
            mark_completed(&state.db, &receipt).await?;
            return Ok(Json(AssistantDeveloperAppResponse {
                resource: AssistantDeveloperAppResource {
                    client_id: receipt.resource_id,
                },
                replayed: true,
                one_time_material: OneTimeMaterialAvailability::Unavailable,
                client_secret: None,
            }));
        }
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let Json(created) = developer_apps::create_my_oauth_client_with_id(
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
        Some(&receipt.resource_id),
    )
    .await?;
    complete_wave4(&state, &receipt, &created.id).await?;
    Ok(Json(AssistantDeveloperAppResponse {
        resource: AssistantDeveloperAppResource {
            client_id: created.id,
        },
        replayed: false,
        one_time_material: OneTimeMaterialAvailability::Delivered,
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
    developer_apps::resolve_developer_app_write_owner(&state, &actor, &id).await?;
    let fp = wave4_fingerprint(&Wave4Fingerprint::DeveloperAppUpdate {
        client_id: &id,
        name: body.name.as_deref(),
        redirect_uris: body.redirect_uris.as_deref(),
    })?;
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
            one_time_material: OneTimeMaterialAvailability::Delivered,
            client_secret: None,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(receipt) => {
            let current = crate::services::oauth_client_service::get_client(&state.db, &id).await?;
            let name_matches = body
                .name
                .as_ref()
                .is_none_or(|name| current.client_name == name.trim());
            let redirects_match = body
                .redirect_uris
                .as_ref()
                .is_none_or(|redirects| current.redirect_uris == *redirects);
            if !name_matches
                || !redirects_match
                || !updated_since_receipt(current.updated_at, &receipt)
            {
                return Err(in_progress_conflict());
            }
            complete_wave4(&state, &receipt, &id).await?;
            return Ok(Json(AssistantDeveloperAppResponse {
                resource: AssistantDeveloperAppResource { client_id: id },
                replayed: true,
                one_time_material: OneTimeMaterialAvailability::Delivered,
                client_secret: None,
            }));
        }
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
        one_time_material: OneTimeMaterialAvailability::Delivered,
        client_secret: None,
    }))
}

async fn delete_developer_app_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantConfirmedDeveloperAppRequest>,
) -> AppResult<Json<AssistantDeveloperAppResponse>> {
    require_destructive_confirmation(body.confirmed)?;
    let actor = auth_user.user_id.to_string();
    let id = parse_uuid(&body.client_id, "clientId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    developer_apps::resolve_developer_app_write_owner(&state, &actor, &id).await?;
    let fp = wave4_fingerprint(&Wave4Fingerprint::DeveloperAppDelete {
        client_id: &id,
        confirmed: body.confirmed,
    })?;
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
            one_time_material: OneTimeMaterialAvailability::Delivered,
            client_secret: None,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(receipt) => {
            let current = crate::services::oauth_client_service::get_client(&state.db, &id).await?;
            if current.is_active || !updated_since_receipt(current.updated_at, &receipt) {
                return Err(in_progress_conflict());
            }
            complete_wave4(&state, &receipt, &id).await?;
            return Ok(Json(AssistantDeveloperAppResponse {
                resource: AssistantDeveloperAppResource { client_id: id },
                replayed: true,
                one_time_material: OneTimeMaterialAvailability::Delivered,
                client_secret: None,
            }));
        }
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let _ =
        developer_apps::delete_my_oauth_client(State(state.clone()), auth_user, Path(id.clone()))
            .await?;
    complete_wave4(&state, &receipt, &id).await?;
    Ok(Json(AssistantDeveloperAppResponse {
        resource: AssistantDeveloperAppResource { client_id: id },
        replayed: false,
        one_time_material: OneTimeMaterialAvailability::Delivered,
        client_secret: None,
    }))
}

async fn rotate_developer_app_secret_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantDeveloperAppRotateRequest>,
) -> AppResult<Json<AssistantDeveloperAppResponse>> {
    let actor = auth_user.user_id.to_string();
    let id = parse_uuid(&body.client_id, "clientId")?;
    let request_id = normalize_action_request_id(body.action_request_id)?;
    developer_apps::resolve_developer_app_write_owner(&state, &actor, &id).await?;
    let expected_updated_at =
        parse_browser_updated_at(&body.expected_updated_at, "expectedUpdatedAt")?;
    let expected_updated_at_fingerprint =
        expected_updated_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let existing_secret_hash = crate::services::oauth_client_service::get_client(&state.db, &id)
        .await?
        .client_secret_hash;
    let secret_fingerprint =
        crate::services::assistant_action_receipts::fingerprint_sensitive_material(
            &existing_secret_hash,
        );
    let fp = wave4_fingerprint(&Wave4Fingerprint::DeveloperAppRotate {
        client_id: &id,
        expected_updated_at: &expected_updated_at_fingerprint,
    })?;
    let outcome = crate::services::assistant_action_receipts::reserve_or_replay_with_secret_marker(
        &state.db,
        &actor,
        DEVELOPER_APP_ROTATE_SECRET_ACTION,
        &request_id,
        &fp,
        id.clone(),
        Some(secret_fingerprint),
    )
    .await?;
    if let Some((_, replayed)) = replayed_resource(&outcome) {
        return Ok(Json(AssistantDeveloperAppResponse {
            resource: AssistantDeveloperAppResource { client_id: id },
            replayed,
            one_time_material: OneTimeMaterialAvailability::Unavailable,
            client_secret: None,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(receipt) => {
            let current = crate::services::oauth_client_service::get_client(&state.db, &id).await?;
            if current.updated_at.timestamp_millis() != expected_updated_at.timestamp_millis() {
                discard_pending_wave4(&state, &receipt).await?;
                return Err(AppError::Conflict(
                    "developer app changed before secret rotation".to_string(),
                ));
            }
            receipt
        }
        ReceiptOutcome::InProgress(receipt) => {
            let current = crate::services::oauth_client_service::get_client(&state.db, &id).await?;
            let committed = receipt
                .resource_secret_fingerprint
                .as_ref()
                .is_some_and(|marker| {
                    crate::services::assistant_action_receipts::fingerprint_sensitive_material(
                        &current.client_secret_hash,
                    ) != *marker
                });
            if !committed {
                return Err(in_progress_conflict());
            }
            mark_completed(&state.db, &receipt).await?;
            return Ok(Json(AssistantDeveloperAppResponse {
                resource: AssistantDeveloperAppResource { client_id: id },
                replayed: true,
                one_time_material: OneTimeMaterialAvailability::Unavailable,
                client_secret: None,
            }));
        }
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
        one_time_material: OneTimeMaterialAvailability::Delivered,
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
    let fp = wave4_fingerprint(&Wave4Fingerprint::NotificationsUpdate {
        telegram_enabled: body.telegram_enabled,
        approval_required: body.approval_required,
        approval_timeout_secs: body.approval_timeout_secs,
        grant_expiry_days: body.grant_expiry_days,
        push_enabled: body.push_enabled,
    })?;
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
        ReceiptOutcome::InProgress(receipt) => {
            let Json(current) =
                notifications::get_settings(State(state.clone()), auth_user.clone()).await?;
            let already_applied = body
                .telegram_enabled
                .is_none_or(|value| current.telegram_enabled == value)
                && body
                    .approval_required
                    .is_none_or(|value| current.approval_required == value)
                && body
                    .approval_timeout_secs
                    .is_none_or(|value| current.approval_timeout_secs == value)
                && body
                    .grant_expiry_days
                    .is_none_or(|value| current.grant_expiry_days == value)
                && body
                    .push_enabled
                    .is_none_or(|value| current.push_enabled == value);
            if !already_applied || !response_updated_since_receipt(&current.updated_at, &receipt)? {
                return Err(in_progress_conflict());
            }
            complete_wave4(&state, &receipt, &current.id).await?;
            return Ok(Json(AssistantNotificationResponse {
                resource: AssistantNotificationBindingResource {
                    binding_id: current.id,
                },
                replayed: true,
            }));
        }
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
    let fp = wave4_fingerprint(&Wave4Fingerprint::NotificationsTelegramLink { user_id: &actor })?;
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
            one_time_material: OneTimeMaterialAvailability::Unavailable,
            link_code: None,
            bot_username: None,
            expires_in_secs: None,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(receipt) => {
            let Json(settings) =
                notifications::get_settings(State(state.clone()), auth_user.clone()).await?;
            if !settings.telegram_link_pending
                || !response_updated_since_receipt(&settings.updated_at, &receipt)?
            {
                return Err(in_progress_conflict());
            }
            mark_completed(&state.db, &receipt).await?;
            return Ok(Json(AssistantTelegramLinkResponse {
                resource: AssistantNotificationBindingResource {
                    binding_id: settings.id,
                },
                replayed: true,
                one_time_material: OneTimeMaterialAvailability::Unavailable,
                link_code: None,
                bot_username: None,
                expires_in_secs: None,
            }));
        }
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
        one_time_material: OneTimeMaterialAvailability::Delivered,
        link_code: Some(link.link_code),
        bot_username: Some(link.bot_username),
        expires_in_secs: Some(link.expires_in_secs),
    }))
}

async fn disconnect_telegram_action(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AssistantConfirmedActionRequestId>,
) -> AppResult<Json<AssistantNotificationResponse>> {
    require_destructive_confirmation(body.confirmed)?;
    let actor = auth_user.user_id.to_string();
    let request_id = normalize_action_request_id(body.action_request_id)?;
    let fp = wave4_fingerprint(&Wave4Fingerprint::NotificationsTelegramDisconnect {
        user_id: &actor,
        confirmed: body.confirmed,
    })?;
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
        ReceiptOutcome::InProgress(receipt) => {
            let Json(current) =
                notifications::get_settings(State(state.clone()), auth_user.clone()).await?;
            if current.telegram_connected
                || current.telegram_enabled
                || current.telegram_link_pending
                || !response_updated_since_receipt(&current.updated_at, &receipt)?
            {
                return Err(in_progress_conflict());
            }
            complete_wave4(&state, &receipt, &current.id).await?;
            return Ok(Json(AssistantNotificationResponse {
                resource: AssistantNotificationBindingResource {
                    binding_id: current.id,
                },
                replayed: true,
            }));
        }
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
            one_time_material: OneTimeMaterialAvailability::Delivered,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(receipt) => {
            if !receipt_resource_exists(
                &state,
                crate::models::user_api_key::COLLECTION_NAME,
                &receipt,
            )
            .await?
            {
                return Err(in_progress_conflict());
            }
            mark_completed(&state.db, &receipt).await?;
            return Ok(Json(AssistantExternalKeyResponse {
                resource: AssistantExternalKeyResource {
                    external_key_id: receipt.resource_id,
                },
                replayed: true,
                one_time_material: OneTimeMaterialAvailability::Delivered,
            }));
        }
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let (_, Json(created)) = user_api_keys_external::create_gcp_service_account_key_with_id(
        State(state.clone()),
        auth_user,
        Json(user_api_keys_external::CreateGcpServiceAccountRequest {
            label: body.label,
            key_json: body.key_json,
            scopes: body.scopes,
            service_slugs: body.service_slugs.unwrap_or_default(),
            target_org_id: body.target_org_id,
        }),
        Some(&receipt.resource_id),
    )
    .await?;
    complete_wave4(&state, &receipt, &created.id).await?;
    Ok(Json(AssistantExternalKeyResponse {
        resource: AssistantExternalKeyResource {
            external_key_id: created.id,
        },
        replayed: false,
        one_time_material: OneTimeMaterialAvailability::Delivered,
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
            one_time_material: OneTimeMaterialAvailability::Delivered,
        }));
    }
    let receipt = match outcome {
        ReceiptOutcome::Reserved(r) => r,
        ReceiptOutcome::InProgress(receipt) => {
            if !receipt_resource_exists(&state, USER_SERVICES, &receipt).await? {
                return Err(in_progress_conflict());
            }
            mark_completed(&state.db, &receipt).await?;
            return Ok(Json(AssistantUserServiceResponse {
                resource: AssistantUserServiceResource {
                    user_service_id: receipt.resource_id,
                },
                replayed: true,
                one_time_material: OneTimeMaterialAvailability::Delivered,
            }));
        }
        ReceiptOutcome::Replay(_) => unreachable!(),
    };
    let Json(created) = keys::create_key_with_service_id(
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
        Some(&receipt.resource_id),
    )
    .await?;
    complete_wave4(&state, &receipt, &created.id).await?;
    Ok(Json(AssistantUserServiceResponse {
        resource: AssistantUserServiceResource {
            user_service_id: created.id,
        },
        replayed: false,
        one_time_material: OneTimeMaterialAvailability::Delivered,
    }))
}

#[cfg(test)]
mod wave4_effect_tests {
    use super::*;
    use chrono::Utc;

    async fn reserve_pending_wave4_receipt(
        db: &mongodb::Database,
        actor_id: &str,
        action: &str,
        request_id: &str,
        fingerprint: &str,
        resource_id: String,
    ) -> crate::models::assistant_action_receipt::AssistantActionReceipt {
        match reserve_or_replay(db, actor_id, action, request_id, fingerprint, resource_id)
            .await
            .expect("reserve receipt")
        {
            ReceiptOutcome::Reserved(receipt) => receipt,
            other => panic!("expected fresh receipt reservation, got {other:?}"),
        }
    }

    async fn reopen_wave4_receipt(
        db: &mongodb::Database,
        actor_id: &str,
        action: &str,
        request_id: &str,
    ) {
        db.collection::<crate::models::assistant_action_receipt::AssistantActionReceipt>(
            crate::models::assistant_action_receipt::COLLECTION_NAME,
        )
        .update_one(
            doc! {
                "user_id": actor_id,
                "action": action,
                "action_request_id": request_id,
            },
            doc! {
                "$set": { "status": "pending" },
                "$unset": { "completed_at": "" },
            },
        )
        .await
        .expect("reopen Wave-4 receipt");
    }

    #[tokio::test]
    async fn mfa_setup_start_interrupted_commit_replays_without_second_factor() {
        let Some(db) =
            crate::test_utils::connect_test_database("assistant_mfa_start_interrupted").await
        else {
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(crate::test_utils::test_user(
                &actor_id,
                crate::models::user::UserType::Person,
            ))
            .await
            .expect("insert actor");
        let state = crate::test_utils::test_app_state(db.clone());
        let first = setup_account_mfa(
            State(state.clone()),
            crate::test_utils::test_auth_user(&actor_id),
            TelemetryContext::default(),
            Json(AssistantMfaSetupRequest {
                action_request_id: "mfa-start-interrupted".to_string(),
                stage: AssistantMfaSetupStage::Start,
                factor_id: None,
                code: None,
            }),
        )
        .await
        .expect("start MFA")
        .0;
        assert!(!first.replayed);
        assert!(first.setup_value.is_some());
        reopen_wave4_receipt(
            &db,
            &actor_id,
            ACCOUNT_MFA_SETUP_START_ACTION,
            "mfa-start-interrupted",
        )
        .await;
        let replay = setup_account_mfa(
            State(state),
            crate::test_utils::test_auth_user(&actor_id),
            TelemetryContext::default(),
            Json(AssistantMfaSetupRequest {
                action_request_id: "mfa-start-interrupted".to_string(),
                stage: AssistantMfaSetupStage::Start,
                factor_id: None,
                code: None,
            }),
        )
        .await
        .expect("recover MFA start")
        .0;
        assert!(replay.replayed);
        assert_eq!(
            replay.one_time_material,
            OneTimeMaterialAvailability::Unavailable
        );
        assert!(replay.setup_value.is_none());
        assert_eq!(
            db.collection::<MfaFactor>(MFA_FACTORS)
                .count_documents(doc! { "user_id": &actor_id, "factor_type": "totp" })
                .await
                .expect("count factors"),
            1
        );
    }

    #[tokio::test]
    async fn org_create_interrupted_commit_replays_reserved_id_without_duplicate() {
        let Some(db) =
            crate::test_utils::connect_test_database("assistant_org_create_reserved").await
        else {
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(crate::test_utils::test_user(
                &actor_id,
                crate::models::user::UserType::Person,
            ))
            .await
            .expect("insert actor");
        let body = AssistantOrgCreateRequest {
            action_request_id: "org-create-reserved".to_string(),
            display_name: "Reserved org".to_string(),
            contact_email: None,
            avatar_url: None,
        };
        let fingerprint = wave4_fingerprint(&Wave4Fingerprint::OrgCreate {
            display_name: &body.display_name,
            contact_email: None,
            avatar_url: None,
        })
        .expect("fingerprint org create");
        let receipt = reserve_pending_wave4_receipt(
            &db,
            &actor_id,
            ORG_CREATE_ACTION,
            &body.action_request_id,
            &fingerprint,
            Uuid::new_v4().to_string(),
        )
        .await;
        let state = crate::test_utils::test_app_state(db.clone());
        let _ = orgs::create_org_with_id(
            State(state.clone()),
            crate::test_utils::test_auth_user(&actor_id),
            Json(orgs::CreateOrgRequest {
                display_name: body.display_name.clone(),
                contact_email: None,
                avatar_url: None,
            }),
            Some(&receipt.resource_id),
        )
        .await
        .expect("commit org create");
        let replay = create_org_action(
            State(state),
            crate::test_utils::test_auth_user(&actor_id),
            Json(body),
        )
        .await
        .expect("recover org create")
        .0;
        assert!(replay.replayed);
        assert_eq!(replay.resource.org_id, receipt.resource_id);
        assert_eq!(
            db.collection::<User>(USERS)
                .count_documents(doc! {"_id": &receipt.resource_id})
                .await
                .expect("count orgs"),
            1
        );
    }

    #[tokio::test]
    async fn service_account_create_interrupted_commit_replays_reserved_id_without_duplicate() {
        let Some(db) =
            crate::test_utils::connect_test_database("assistant_sa_create_reserved").await
        else {
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                crate::test_utils::test_user(&actor_id, crate::models::user::UserType::Person),
                crate::test_utils::test_user(&org_id, crate::models::user::UserType::Org),
            ])
            .await
            .expect("insert users");
        db.collection::<crate::models::org_membership::OrgMembership>(
            crate::models::org_membership::COLLECTION_NAME,
        )
        .insert_one(crate::test_utils::test_membership(
            &org_id,
            &actor_id,
            crate::models::org_membership::OrgRole::Admin,
            None,
        ))
        .await
        .expect("insert membership");
        let body = AssistantServiceAccountCreateRequest {
            action_request_id: "sa-create-reserved".to_string(),
            name: "Reserved SA".to_string(),
            description: None,
            allowed_scopes: Some("proxy".to_string()),
            target_org_id: Some(org_id.clone()),
        };
        let fingerprint =
            service_account_create_fingerprint(&body.name, None, Some("proxy"), Some(&org_id))
                .expect("fingerprint service account create");
        let receipt = reserve_pending_wave4_receipt(
            &db,
            &actor_id,
            SERVICE_ACCOUNT_CREATE_ACTION,
            &body.action_request_id,
            &fingerprint,
            Uuid::new_v4().to_string(),
        )
        .await;
        let state = crate::test_utils::test_app_state(db.clone());
        let _ = admin_service_accounts::create_service_account_with_id(
            State(state.clone()),
            crate::test_utils::test_auth_user(&actor_id),
            TelemetryContext::default(),
            HeaderMap::new(),
            Json(admin_service_accounts::CreateServiceAccountRequest {
                name: body.name.clone(),
                description: None,
                allowed_scopes: "proxy".to_string(),
                role_ids: None,
                rate_limit_override: None,
                target_org_id: Some(org_id),
            }),
            Some(&receipt.resource_id),
        )
        .await
        .expect("commit service-account create");
        let replay = create_service_account_action(
            State(state),
            crate::test_utils::test_auth_user(&actor_id),
            Json(body),
        )
        .await
        .expect("recover service-account create")
        .0;
        assert!(replay.replayed);
        assert_eq!(replay.resource.service_account_id, receipt.resource_id);
        assert_eq!(
            db.collection::<crate::models::service_account::ServiceAccount>(
                crate::models::service_account::COLLECTION_NAME
            )
            .count_documents(doc! {"_id": &receipt.resource_id})
            .await
            .expect("count service accounts"),
            1
        );
    }

    #[tokio::test]
    async fn developer_app_create_interrupted_commit_replays_reserved_id_without_duplicate() {
        let Some(db) =
            crate::test_utils::connect_test_database("assistant_app_create_reserved").await
        else {
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(crate::test_utils::test_user(
                &actor_id,
                crate::models::user::UserType::Person,
            ))
            .await
            .expect("insert actor");
        let body = AssistantDeveloperAppCreateRequest {
            action_request_id: "app-create-reserved".to_string(),
            name: "Reserved app".to_string(),
            redirect_uris: vec!["https://example.test/callback".to_string()],
        };
        let fingerprint = wave4_fingerprint(&Wave4Fingerprint::DeveloperAppCreate {
            name: &body.name,
            redirect_uris: &body.redirect_uris,
        })
        .expect("fingerprint developer app create");
        let receipt = reserve_pending_wave4_receipt(
            &db,
            &actor_id,
            DEVELOPER_APP_CREATE_ACTION,
            &body.action_request_id,
            &fingerprint,
            Uuid::new_v4().to_string(),
        )
        .await;
        let state = crate::test_utils::test_app_state(db.clone());
        let _ = developer_apps::create_my_oauth_client_with_id(
            State(state.clone()),
            crate::test_utils::test_auth_user(&actor_id),
            TelemetryContext::default(),
            Json(developer_apps::CreateDeveloperOAuthClientRequest {
                name: body.name.clone(),
                redirect_uris: body.redirect_uris.clone(),
                client_type: None,
                delegation_scopes: None,
                broker_capability_enabled: None,
                revocation_webhook_url: None,
                revocation_webhook_secret: None,
                allowed_scopes: None,
                target_org_id: None,
                default_service_catalog_slugs: None,
            }),
            Some(&receipt.resource_id),
        )
        .await
        .expect("commit developer app create");
        let replay = create_developer_app_action(
            State(state),
            crate::test_utils::test_auth_user(&actor_id),
            Json(body),
        )
        .await
        .expect("recover developer app create")
        .0;
        assert!(replay.replayed);
        assert_eq!(replay.resource.client_id, receipt.resource_id);
        assert_eq!(
            db.collection::<crate::models::oauth_client::OauthClient>(
                crate::models::oauth_client::COLLECTION_NAME
            )
            .count_documents(doc! {"_id": &receipt.resource_id})
            .await
            .expect("count apps"),
            1
        );
    }

    #[tokio::test]
    async fn gcp_key_create_interrupted_commit_replays_reserved_id_without_duplicate() {
        let Some(db) =
            crate::test_utils::connect_test_database("assistant_gcp_create_reserved").await
        else {
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(crate::test_utils::test_user(
                &actor_id,
                crate::models::user::UserType::Person,
            ))
            .await
            .expect("insert actor");
        let body = AssistantGcpCreateRequest {
            action_request_id: "gcp-create-reserved".to_string(),
            label: Some("Reserved GCP".to_string()),
            key_json: "{\"client_email\":\"svc@example.test\",\"private_key\":\"redacted\"}"
                .to_string(),
            scopes: None,
            service_slugs: None,
            target_org_id: None,
        };
        let fingerprint = gcp_create_fingerprint(&body).expect("fingerprint GCP create");
        let receipt = reserve_pending_wave4_receipt(
            &db,
            &actor_id,
            EXTERNAL_KEY_ADD_GCP_ACTION,
            &body.action_request_id,
            &fingerprint,
            Uuid::new_v4().to_string(),
        )
        .await;
        let state = crate::test_utils::test_app_state(db.clone());
        crate::services::user_api_key_service::create_api_key_with_id(
            &db,
            &state.encryption_keys,
            &actor_id,
            &receipt.resource_id,
            crate::services::user_api_key_service::CreateApiKeyParams {
                label: "Reserved GCP",
                credential_type: "gcp_service_account",
                credential: &body.key_json,
                access_token: None,
                refresh_token: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: None,
                connection_id: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                status: "active",
                source: Some("user_created"),
                source_id: None,
            },
        )
        .await
        .expect("commit GCP key create");
        let replay = add_gcp_service_account_action(
            State(state),
            crate::test_utils::test_auth_user(&actor_id),
            Json(body),
        )
        .await
        .expect("recover GCP key create")
        .0;
        assert!(replay.replayed);
        assert_eq!(replay.resource.external_key_id, receipt.resource_id);
        assert_eq!(
            db.collection::<crate::models::user_api_key::UserApiKey>(
                crate::models::user_api_key::COLLECTION_NAME
            )
            .count_documents(doc! {"_id": &receipt.resource_id})
            .await
            .expect("count GCP keys"),
            1
        );
    }

    #[tokio::test]
    async fn openclaw_connect_interrupted_commit_replays_reserved_id_without_duplicate() {
        let Some(db) =
            crate::test_utils::connect_test_database("assistant_openclaw_reserved").await
        else {
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(crate::test_utils::test_user(
                &actor_id,
                crate::models::user::UserType::Person,
            ))
            .await
            .expect("insert actor");
        let body = AssistantOpenClawConnectRequest {
            action_request_id: "openclaw-reserved".to_string(),
            gateway_url: "https://openclaw.example.test".to_string(),
            credential: "bearer-token".to_string(),
            label: Some("Reserved OpenClaw".to_string()),
        };
        let fingerprint = openclaw_connect_fingerprint(
            &body.gateway_url,
            &body.credential,
            body.label.as_deref(),
        )
        .expect("fingerprint OpenClaw connect");
        let receipt = reserve_pending_wave4_receipt(
            &db,
            &actor_id,
            OPENCLAW_CONNECT_ACTION,
            &body.action_request_id,
            &fingerprint,
            Uuid::new_v4().to_string(),
        )
        .await;
        let state = crate::test_utils::test_app_state(db.clone());
        crate::services::provider_service::seed_default_providers(&db, &state.encryption_keys)
            .await
            .expect("seed OpenClaw provider");
        crate::services::provider_service::seed_default_services(&db, &state.encryption_keys)
            .await
            .expect("seed OpenClaw catalog service");
        let _ = keys::create_key_with_service_id(
            State(state.clone()),
            crate::test_utils::test_auth_user(&actor_id),
            TelemetryContext::default(),
            Json(keys::CreateKeyRequest {
                service_slug: Some("llm-openclaw".to_string()),
                credential: Some(body.credential.clone()),
                label: body.label.clone().unwrap_or_else(|| "OpenClaw".to_string()),
                endpoint_url: Some(body.gateway_url.clone()),
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
            Some(&receipt.resource_id),
        )
        .await
        .expect("commit OpenClaw service create");
        let replay = connect_openclaw_action(
            State(state),
            crate::test_utils::test_auth_user(&actor_id),
            Json(body),
        )
        .await
        .expect("recover OpenClaw connect")
        .0;
        assert!(replay.replayed);
        assert_eq!(replay.resource.user_service_id, receipt.resource_id);
        assert_eq!(
            db.collection::<UserService>(USER_SERVICES)
                .count_documents(doc! {"_id": &receipt.resource_id})
                .await
                .expect("count OpenClaw services"),
            1
        );
    }

    #[tokio::test]
    async fn developer_app_rotate_interrupted_commit_replays_without_second_rotation() {
        let Some(db) =
            crate::test_utils::connect_test_database("assistant_app_rotate_interrupted").await
        else {
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(crate::test_utils::test_user(
                &actor_id,
                crate::models::user::UserType::Person,
            ))
            .await
            .expect("insert actor");
        let state = crate::test_utils::test_app_state(db.clone());
        let created = developer_apps::create_my_oauth_client(
            State(state.clone()),
            crate::test_utils::test_auth_user(&actor_id),
            TelemetryContext::default(),
            Json(developer_apps::CreateDeveloperOAuthClientRequest {
                name: "Interrupted rotation".to_string(),
                redirect_uris: vec!["https://example.test/callback".to_string()],
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
        .await
        .expect("create app")
        .0;
        let before = crate::services::oauth_client_service::get_client(&db, &created.id)
            .await
            .expect("load app");
        let expected_updated_at = before.updated_at.to_rfc3339();
        let request = || AssistantDeveloperAppRotateRequest {
            action_request_id: "app-rotate-interrupted".to_string(),
            client_id: created.id.clone(),
            expected_updated_at: expected_updated_at.clone(),
        };
        let first = rotate_developer_app_secret_action(
            State(state.clone()),
            crate::test_utils::test_auth_user(&actor_id),
            Json(request()),
        )
        .await
        .expect("rotate app")
        .0;
        assert!(!first.replayed);
        let rotated_hash = crate::services::oauth_client_service::get_client(&db, &created.id)
            .await
            .expect("load rotated app")
            .client_secret_hash;
        reopen_wave4_receipt(
            &db,
            &actor_id,
            DEVELOPER_APP_ROTATE_SECRET_ACTION,
            "app-rotate-interrupted",
        )
        .await;
        let replay = rotate_developer_app_secret_action(
            State(state),
            crate::test_utils::test_auth_user(&actor_id),
            Json(request()),
        )
        .await
        .expect("recover app rotation")
        .0;
        assert!(replay.replayed);
        assert_eq!(
            replay.one_time_material,
            OneTimeMaterialAvailability::Unavailable
        );
        assert_eq!(
            crate::services::oauth_client_service::get_client(&db, &created.id)
                .await
                .expect("reload app")
                .client_secret_hash,
            rotated_hash,
            "retry must not rotate the client secret twice"
        );
    }

    #[tokio::test]
    async fn telegram_link_interrupted_commit_replays_without_second_code() {
        let Some(db) =
            crate::test_utils::connect_test_database("assistant_telegram_link_interrupted").await
        else {
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(crate::test_utils::test_user(
                &actor_id,
                crate::models::user::UserType::Person,
            ))
            .await
            .expect("insert actor");
        let state = crate::test_utils::test_app_state(db.clone());
        let first = link_telegram_action(
            State(state.clone()),
            crate::test_utils::test_auth_user(&actor_id),
            Json(AssistantActionRequestId {
                action_request_id: "telegram-link-interrupted".to_string(),
            }),
        )
        .await
        .expect("link Telegram")
        .0;
        assert!(!first.replayed);
        let channel_before = db
            .collection::<crate::models::notification_channel::NotificationChannel>(
                crate::models::notification_channel::COLLECTION_NAME,
            )
            .find_one(doc! { "user_id": &actor_id })
            .await
            .expect("load notification channel")
            .expect("notification channel");
        reopen_wave4_receipt(
            &db,
            &actor_id,
            NOTIFICATIONS_TELEGRAM_LINK_ACTION,
            "telegram-link-interrupted",
        )
        .await;
        let replay = link_telegram_action(
            State(state),
            crate::test_utils::test_auth_user(&actor_id),
            Json(AssistantActionRequestId {
                action_request_id: "telegram-link-interrupted".to_string(),
            }),
        )
        .await
        .expect("recover Telegram link")
        .0;
        assert!(replay.replayed);
        assert_eq!(
            replay.one_time_material,
            OneTimeMaterialAvailability::Unavailable
        );
        assert!(replay.link_code.is_none());
        let channel_after = db
            .collection::<crate::models::notification_channel::NotificationChannel>(
                crate::models::notification_channel::COLLECTION_NAME,
            )
            .find_one(doc! { "user_id": &actor_id })
            .await
            .expect("reload notification channel")
            .expect("notification channel");
        assert_eq!(
            channel_after.telegram_link_code, channel_before.telegram_link_code,
            "retry must not mint a second Telegram link code"
        );
    }

    #[test]
    fn replay_material_serialization_distinguishes_secret_and_non_secret_actions() {
        let secret_replay = serde_json::to_value(AssistantServiceAccountResponse {
            resource: AssistantServiceAccountResource {
                service_account_id: "sa-1".to_string(),
            },
            replayed: true,
            one_time_material: OneTimeMaterialAvailability::Unavailable,
            client_secret: None,
        })
        .expect("serialize secret replay");
        assert_eq!(secret_replay["oneTimeMaterial"], "unavailable");

        let non_secret_replay = serde_json::to_value(AssistantUserServiceResponse {
            resource: AssistantUserServiceResource {
                user_service_id: "service-1".to_string(),
            },
            replayed: true,
            one_time_material: OneTimeMaterialAvailability::Delivered,
        })
        .expect("serialize non-secret replay");
        assert_eq!(non_secret_replay["oneTimeMaterial"], "delivered");
        assert_ne!(
            non_secret_replay["oneTimeMaterial"], "unavailable",
            "non-material actions must not warn that a secret was lost"
        );
    }

    #[tokio::test]
    async fn service_account_rotation_metadata_bump_is_not_proof_of_commit() {
        let Some(db) =
            crate::test_utils::connect_test_database("assistant_sa_rotation_marker").await
        else {
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &actor_id,
                crate::models::user::UserType::Person,
            ))
            .await
            .expect("insert actor");
        let (service_account, _) =
            crate::services::service_account_service::create_service_account(
                &db,
                "marker test",
                None,
                "proxy",
                &[],
                None,
                &actor_id,
            )
            .await
            .expect("create service account");
        let expected = service_account
            .updated_at
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let marker = crate::services::assistant_action_receipts::fingerprint_sensitive_material(
            &service_account.client_secret_hash,
        );
        let fingerprint = wave4_fingerprint(&Wave4Fingerprint::ServiceAccountRotate {
            service_account_id: &service_account.id,
            expected_updated_at: &expected,
        })
        .expect("fingerprint rotation");
        assert!(matches!(
            crate::services::assistant_action_receipts::reserve_or_replay_with_secret_marker(
                &db,
                &actor_id,
                SERVICE_ACCOUNT_ROTATE_SECRET_ACTION,
                "sa-marker-retry",
                &fingerprint,
                service_account.id.clone(),
                Some(marker),
            )
            .await
            .expect("reserve receipt"),
            ReceiptOutcome::Reserved(_)
        ));
        db.collection::<crate::models::service_account::ServiceAccount>(
            crate::models::service_account::COLLECTION_NAME,
        )
        .update_one(
            doc! { "_id": &service_account.id },
            doc! { "$set": { "updated_at": bson::DateTime::from_chrono(Utc::now() + chrono::Duration::seconds(1)) } },
        )
        .await
        .expect("bump metadata timestamp");

        let result = rotate_service_account_secret_action(
            State(crate::test_utils::test_app_state(db)),
            crate::test_utils::test_auth_user(&actor_id),
            Json(AssistantServiceAccountRotateRequest {
                action_request_id: "sa-marker-retry".to_string(),
                service_account_id: service_account.id,
                expected_updated_at: expected,
            }),
        )
        .await;
        assert!(matches!(result, Err(AppError::Conflict(_))));
    }

    #[tokio::test]
    async fn developer_app_rotation_metadata_bump_is_not_proof_of_commit() {
        let Some(db) =
            crate::test_utils::connect_test_database("assistant_app_rotation_marker").await
        else {
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &actor_id,
                crate::models::user::UserType::Person,
            ))
            .await
            .expect("insert actor");
        let (client, _) = crate::services::oauth_client_service::create_client(
            &db,
            "marker app",
            &["https://example.test/callback".to_string()],
            "confidential",
            &actor_id,
            "",
            crate::services::oauth_client_service::DEFAULT_ALLOWED_SCOPES,
            crate::models::oauth_client::ScopeProvenance::Defaulted,
            false,
            None,
            None,
            &[],
        )
        .await
        .expect("create developer app");
        let expected = client
            .updated_at
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let marker = crate::services::assistant_action_receipts::fingerprint_sensitive_material(
            &client.client_secret_hash,
        );
        let fingerprint = wave4_fingerprint(&Wave4Fingerprint::DeveloperAppRotate {
            client_id: &client.id,
            expected_updated_at: &expected,
        })
        .expect("fingerprint rotation");
        assert!(matches!(
            crate::services::assistant_action_receipts::reserve_or_replay_with_secret_marker(
                &db,
                &actor_id,
                DEVELOPER_APP_ROTATE_SECRET_ACTION,
                "app-marker-retry",
                &fingerprint,
                client.id.clone(),
                Some(marker),
            )
            .await
            .expect("reserve receipt"),
            ReceiptOutcome::Reserved(_)
        ));
        db.collection::<crate::models::oauth_client::OauthClient>(
            crate::models::oauth_client::COLLECTION_NAME,
        )
        .update_one(
            doc! { "_id": &client.id },
            doc! { "$set": { "updated_at": bson::DateTime::from_chrono(Utc::now() + chrono::Duration::seconds(1)) } },
        )
        .await
        .expect("bump metadata timestamp");

        let result = rotate_developer_app_secret_action(
            State(crate::test_utils::test_app_state(db)),
            crate::test_utils::test_auth_user(&actor_id),
            Json(AssistantDeveloperAppRotateRequest {
                action_request_id: "app-marker-retry".to_string(),
                client_id: client.id,
                expected_updated_at: expected,
            }),
        )
        .await;
        assert!(matches!(result, Err(AppError::Conflict(_))));
    }

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
        use axum::http::Request;
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
        assert_eq!(
            db.collection::<mongodb::bson::Document>(
                crate::models::assistant_action_receipt::COLLECTION_NAME,
            )
            .count_documents(doc! {})
            .await
            .expect("count refused delete receipts"),
            0,
            "a refused confirmation must not reserve a receipt"
        );
    }

    #[tokio::test]
    async fn destructive_preconditions_are_checked_before_reservation() {
        // Falsifier: accept `confirmed: false` or move any confirmation check
        // below `reserve_or_replay`; that case either stops returning a
        // validation error or leaves a pending receipt counted at the end.
        use crate::models::assistant_action_receipt::COLLECTION_NAME as ASSISTANT_ACTION_RECEIPTS;
        use crate::test_utils::{connect_test_database, test_app_state, test_auth_user};

        let Some(db) = connect_test_database("assistant_destructive_preconditions").await else {
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        let resource_id = Uuid::new_v4().to_string();
        let state = test_app_state(db.clone());

        let consent = revoke_account_consent(
            State(state.clone()),
            test_auth_user(&actor_id),
            Json(RevokeAssistantConsentRequest {
                action_request_id: "refuse-consent".to_string(),
                client_id: resource_id.clone(),
                confirmed: false,
            }),
        )
        .await;
        assert!(matches!(consent, Err(AppError::ValidationError(_))));

        let org_delete = delete_org_action(
            State(state.clone()),
            test_auth_user(&actor_id),
            Json(AssistantConfirmedOrgIdRequest {
                action_request_id: "refuse-org-delete".to_string(),
                org_id: resource_id.clone(),
                confirmed: false,
            }),
        )
        .await;
        assert!(matches!(org_delete, Err(AppError::ValidationError(_))));

        let member_remove = remove_org_member_action(
            State(state.clone()),
            test_auth_user(&actor_id),
            Json(AssistantConfirmedOrgMemberRequest {
                action_request_id: "refuse-member-remove".to_string(),
                org_id: resource_id.clone(),
                member_id: Uuid::new_v4().to_string(),
                confirmed: false,
            }),
        )
        .await;
        assert!(matches!(member_remove, Err(AppError::ValidationError(_))));

        for (action_request_id, result) in [
            (
                "refuse-service-account-delete",
                delete_service_account_action(
                    State(state.clone()),
                    test_auth_user(&actor_id),
                    Json(AssistantConfirmedServiceAccountRequest {
                        action_request_id: "refuse-service-account-delete".to_string(),
                        service_account_id: resource_id.clone(),
                        confirmed: false,
                    }),
                )
                .await,
            ),
            (
                "refuse-service-account-tokens",
                revoke_service_account_tokens_action(
                    State(state.clone()),
                    test_auth_user(&actor_id),
                    Json(AssistantConfirmedServiceAccountRequest {
                        action_request_id: "refuse-service-account-tokens".to_string(),
                        service_account_id: resource_id.clone(),
                        confirmed: false,
                    }),
                )
                .await,
            ),
        ] {
            assert!(
                matches!(result, Err(AppError::ValidationError(_))),
                "{action_request_id} was not refused"
            );
        }

        let developer_delete = delete_developer_app_action(
            State(state.clone()),
            test_auth_user(&actor_id),
            Json(AssistantConfirmedDeveloperAppRequest {
                action_request_id: "refuse-developer-delete".to_string(),
                client_id: resource_id,
                confirmed: false,
            }),
        )
        .await;
        assert!(matches!(
            developer_delete,
            Err(AppError::ValidationError(_))
        ));

        let telegram_disconnect = disconnect_telegram_action(
            State(state),
            test_auth_user(&actor_id),
            Json(AssistantConfirmedActionRequestId {
                action_request_id: "refuse-telegram-disconnect".to_string(),
                confirmed: false,
            }),
        )
        .await;
        assert!(matches!(
            telegram_disconnect,
            Err(AppError::ValidationError(_))
        ));

        assert_eq!(
            db.collection::<mongodb::bson::Document>(ASSISTANT_ACTION_RECEIPTS)
                .count_documents(doc! {})
                .await
                .expect("count destructive receipts"),
            0
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

    #[tokio::test]
    async fn mfa_confirm_resume_requires_the_pinned_factor() {
        // Falsifier: resume from `user.mfa_enabled` alone. The first retry
        // below then succeeds even though its fingerprinted factor is still
        // unverified, and this test fails before the factor update.
        use crate::models::assistant_action_receipt::{
            AssistantActionReceipt, AssistantActionReceiptStatus,
            COLLECTION_NAME as ASSISTANT_ACTION_RECEIPTS,
        };
        use crate::models::user::{COLLECTION_NAME as USERS, UserType};
        use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};

        let Some(db) = connect_test_database("assistant_mfa_pinned_resume").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let factor_id = Uuid::new_v4().to_string();
        let mut user = test_user(&user_id, UserType::Person);
        // This can be explained by another already-confirmed factor. It is
        // deliberately insufficient evidence for this action's factor.
        user.mfa_enabled = true;
        db.collection::<User>(USERS)
            .insert_one(user)
            .await
            .expect("insert user");
        let now = chrono::Utc::now();
        db.collection::<MfaFactor>(MFA_FACTORS)
            .insert_one(MfaFactor {
                id: factor_id.clone(),
                user_id: user_id.clone(),
                factor_type: "totp".to_string(),
                secret_encrypted: None,
                recovery_codes: None,
                is_verified: false,
                is_active: true,
                created_at: now,
                updated_at: now,
            })
            .await
            .expect("insert pinned factor");

        let action_request_id = "mfa-confirm-interrupted";
        let fingerprint = fingerprint_canonical(&MfaFingerprint {
            action: ACCOUNT_MFA_SETUP_ACTION,
            user_id: &user_id,
            stage: AssistantMfaSetupStage::Confirm,
            factor_id: Some(&factor_id),
        })
        .expect("fingerprint");
        assert!(matches!(
            reserve_or_replay(
                &db,
                &user_id,
                ACCOUNT_MFA_SETUP_ACTION,
                action_request_id,
                &fingerprint,
                user_id.clone(),
            )
            .await
            .expect("reserve receipt"),
            ReceiptOutcome::Reserved(_)
        ));

        let state = test_app_state(db.clone());
        let request = || AssistantMfaSetupRequest {
            action_request_id: action_request_id.to_string(),
            stage: AssistantMfaSetupStage::Confirm,
            factor_id: Some(factor_id.clone()),
            code: Some("123456".to_string()),
        };
        let denied = setup_account_mfa(
            State(state.clone()),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Json(request()),
        )
        .await;
        assert!(matches!(denied, Err(AppError::Conflict(_))));

        let receipt = db
            .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
            .find_one(doc! { "action_request_id": action_request_id })
            .await
            .expect("load pending receipt")
            .expect("pending receipt");
        assert_eq!(receipt.status, AssistantActionReceiptStatus::Pending);

        db.collection::<MfaFactor>(MFA_FACTORS)
            .update_one(
                doc! { "_id": &factor_id },
                doc! { "$set": { "is_verified": true } },
            )
            .await
            .expect("verify pinned factor");
        let resumed = setup_account_mfa(
            State(state),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Json(request()),
        )
        .await
        .expect("resume after pinned factor confirmation");
        assert!(resumed.0.replayed);
        assert_eq!(resumed.0.factor_id.as_deref(), Some(factor_id.as_str()));

        let completed = db
            .collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS)
            .find_one(doc! { "action_request_id": action_request_id })
            .await
            .expect("load completed receipt")
            .expect("completed receipt");
        assert_eq!(completed.status, AssistantActionReceiptStatus::Completed);
    }

    #[tokio::test]
    async fn member_role_updates_cannot_exceed_actor_authority() {
        // Falsifiers: remove `require_delegable_scope` from
        // `authorize_member_update` and the scoped admin grants unrestricted
        // admin authority; remove the pre-reservation access check and the
        // denied attempts leave pending receipts behind.
        use crate::models::assistant_action_receipt::COLLECTION_NAME as ASSISTANT_ACTION_RECEIPTS;
        use crate::models::oauth_client::ScopeProvenance;
        use crate::models::org_membership::{
            COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
        };
        use crate::models::user::{COLLECTION_NAME as USERS, UserType};
        use crate::services::oauth_client_service;
        use crate::test_utils::{
            connect_test_database, test_app_state, test_auth_user, test_membership, test_user,
        };

        let Some(db) = connect_test_database("assistant_member_role_authority").await else {
            return;
        };
        let org_id = Uuid::new_v4().to_string();
        let admin_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        let nonactor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&org_id, UserType::Org),
                test_user(&admin_id, UserType::Person),
                test_user(&member_id, UserType::Person),
                test_user(&nonactor_id, UserType::Person),
            ])
            .await
            .expect("insert authority users");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_many([
                test_membership(
                    &org_id,
                    &admin_id,
                    OrgRole::Admin,
                    Some(vec!["svc-a".to_string()]),
                ),
                test_membership(&org_id, &member_id, OrgRole::Member, None),
            ])
            .await
            .expect("insert authority memberships");

        let state = test_app_state(db.clone());
        let request = |action_request_id: &str, target: &str| AssistantOrgMemberRoleRequest {
            action_request_id: action_request_id.to_string(),
            org_id: org_id.clone(),
            member_id: target.to_string(),
            role: orgs::OrgRoleWire::Admin,
            expected_role: orgs::OrgRoleWire::Member,
        };

        let widened = update_org_member_role_action(
            State(state.clone()),
            test_auth_user(&admin_id),
            Json(request("member-role-widen", &member_id)),
        )
        .await;
        assert!(matches!(widened, Err(AppError::OrgRoleInsufficient(_))));

        let self_escalation = update_org_member_role_action(
            State(state.clone()),
            test_auth_user(&member_id),
            Json(request("member-role-self", &member_id)),
        )
        .await;
        assert!(matches!(
            self_escalation,
            Err(AppError::OrgRoleInsufficient(_))
        ));

        let unrelated = update_org_member_role_action(
            State(state.clone()),
            test_auth_user(&nonactor_id),
            Json(request("member-role-nonactor", &member_id)),
        )
        .await;
        assert!(matches!(unrelated, Err(AppError::NotFound(_))));

        let (service_account, _) = service_account_service::create_service_account(
            &db,
            "owner-service-account",
            None,
            "proxy",
            &[],
            None,
            &admin_id,
        )
        .await
        .expect("create owned service account");
        let (developer_app, _) = oauth_client_service::create_client(
            &db,
            "Owner application",
            &["https://owner.example/callback".to_string()],
            "confidential",
            &admin_id,
            "",
            oauth_client_service::DEFAULT_ALLOWED_SCOPES,
            ScopeProvenance::Explicit,
            false,
            None,
            None,
            &[],
        )
        .await
        .expect("create owned developer app");

        let service_account_denied = update_service_account_action(
            State(state.clone()),
            test_auth_user(&nonactor_id),
            Json(AssistantServiceAccountUpdateRequest {
                action_request_id: "service-account-nonactor".to_string(),
                service_account_id: service_account.id.clone(),
                name: Some("unauthorized rename".to_string()),
                description: None,
            }),
        )
        .await;
        assert!(matches!(service_account_denied, Err(AppError::NotFound(_))));

        let developer_app_denied = update_developer_app_action(
            State(state),
            test_auth_user(&nonactor_id),
            Json(AssistantDeveloperAppUpdateRequest {
                action_request_id: "developer-app-nonactor".to_string(),
                client_id: developer_app.id.clone(),
                name: Some("unauthorized rename".to_string()),
                redirect_uris: None,
            }),
        )
        .await;
        assert!(matches!(developer_app_denied, Err(AppError::NotFound(_))));

        let member = db
            .collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .find_one(doc! { "org_user_id": &org_id, "member_user_id": &member_id })
            .await
            .expect("load target membership")
            .expect("target membership");
        assert_eq!(member.role, OrgRole::Member);
        let stored_service_account =
            service_account_service::get_service_account(&db, &service_account.id)
                .await
                .expect("load service account");
        assert_eq!(stored_service_account.name, "owner-service-account");
        let stored_developer_app = oauth_client_service::get_client(&db, &developer_app.id)
            .await
            .expect("load developer app");
        assert_eq!(stored_developer_app.client_name, "Owner application");
        assert_eq!(
            db.collection::<mongodb::bson::Document>(ASSISTANT_ACTION_RECEIPTS)
                .count_documents(doc! {})
                .await
                .expect("count receipts"),
            0,
            "authority denials must happen before receipt reservation"
        );
    }

    #[tokio::test]
    async fn receipts_prevent_duplicate_effects_and_replayed_one_time_material() {
        // Falsifiers:
        // - Return the create/rotate material from a completed `Replay`; the
        //   four `client_secret.is_none()` assertions fail.
        // - Route a rotation `InProgress` receipt into its mutation arm; the
        //   reopened retry changes the persisted service-account secret hash.
        // - Remove the `expectedUpdatedAt` comparison or its pending-receipt
        //   discard; the stale rotation changes the hash or leaves a receipt.
        // - Restore the unconditional notifications `InProgress` conflict or
        //   rerun its mutation; the resumed call fails or advances `updated_at`.
        use crate::models::assistant_action_receipt::{
            AssistantActionReceipt, AssistantActionReceiptStatus,
            COLLECTION_NAME as ASSISTANT_ACTION_RECEIPTS,
        };
        use crate::models::notification_channel::{
            COLLECTION_NAME as NOTIFICATION_CHANNELS, NotificationChannel,
        };
        use crate::models::org_membership::{
            COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
        };
        use crate::models::user::{COLLECTION_NAME as USERS, UserType};
        use crate::services::oauth_client_service;
        use crate::test_utils::{
            connect_test_database, test_app_state, test_auth_user, test_membership, test_user,
        };

        let Some(db) = connect_test_database("assistant_receipt_material_discipline").await else {
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&actor_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .expect("insert receipt-discipline users");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &actor_id, OrgRole::Admin, None))
            .await
            .expect("insert org admin membership");

        let state = test_app_state(db.clone());
        let auth = || test_auth_user(&actor_id);

        let service_account_create = || AssistantServiceAccountCreateRequest {
            action_request_id: "service-account-create-once".to_string(),
            name: "assistant receipt service account".to_string(),
            description: Some("receipt discipline fixture".to_string()),
            allowed_scopes: Some("proxy".to_string()),
            target_org_id: Some(org_id.clone()),
        };
        let created_service_account = create_service_account_action(
            State(state.clone()),
            auth(),
            Json(service_account_create()),
        )
        .await
        .expect("create service account")
        .0;
        assert!(!created_service_account.replayed);
        assert!(created_service_account.client_secret.is_some());
        let service_account_id = created_service_account.resource.service_account_id;

        let replayed_service_account = create_service_account_action(
            State(state.clone()),
            auth(),
            Json(service_account_create()),
        )
        .await
        .expect("replay service-account create")
        .0;
        assert!(replayed_service_account.replayed);
        assert!(replayed_service_account.client_secret.is_none());
        assert_eq!(
            replayed_service_account.one_time_material,
            OneTimeMaterialAvailability::Unavailable
        );
        assert_eq!(
            db.collection::<mongodb::bson::Document>(
                crate::models::service_account::COLLECTION_NAME,
            )
            .count_documents(doc! {
                "_id": &service_account_id,
                "owner_user_id": &org_id,
            })
            .await
            .expect("count created service account"),
            1
        );

        let service_account_before =
            service_account_service::get_service_account(&db, &service_account_id)
                .await
                .expect("load service account before rotation");
        let service_account_expected_at = service_account_before.updated_at.to_rfc3339();
        let service_account_rotate = || AssistantServiceAccountRotateRequest {
            action_request_id: "service-account-rotate-once".to_string(),
            service_account_id: service_account_id.clone(),
            expected_updated_at: service_account_expected_at.clone(),
        };
        let rotated_service_account = rotate_service_account_secret_action(
            State(state.clone()),
            auth(),
            Json(service_account_rotate()),
        )
        .await
        .expect("rotate service-account secret")
        .0;
        assert!(!rotated_service_account.replayed);
        assert!(rotated_service_account.client_secret.is_some());
        let service_account_after =
            service_account_service::get_service_account(&db, &service_account_id)
                .await
                .expect("load rotated service account");
        let rotated_service_account_hash = service_account_after.client_secret_hash.clone();
        assert_ne!(
            rotated_service_account_hash,
            service_account_before.client_secret_hash
        );

        let replayed_service_account_rotation = rotate_service_account_secret_action(
            State(state.clone()),
            auth(),
            Json(service_account_rotate()),
        )
        .await
        .expect("replay completed service-account rotation")
        .0;
        assert!(replayed_service_account_rotation.replayed);
        assert!(replayed_service_account_rotation.client_secret.is_none());

        let receipts = db.collection::<AssistantActionReceipt>(ASSISTANT_ACTION_RECEIPTS);
        receipts
            .update_one(
                doc! {
                    "user_id": &actor_id,
                    "action": SERVICE_ACCOUNT_ROTATE_SECRET_ACTION,
                    "action_request_id": "service-account-rotate-once",
                },
                doc! {
                    "$set": { "status": "pending" },
                    "$unset": { "completed_at": "" },
                },
            )
            .await
            .expect("reopen service-account rotation receipt");
        let interrupted_service_account_retry = rotate_service_account_secret_action(
            State(state.clone()),
            auth(),
            Json(service_account_rotate()),
        )
        .await;
        let interrupted_service_account_retry = interrupted_service_account_retry
            .expect("resume committed service-account rotation")
            .0;
        assert!(interrupted_service_account_retry.replayed);
        assert_eq!(
            interrupted_service_account_retry.one_time_material,
            OneTimeMaterialAvailability::Unavailable
        );
        assert!(interrupted_service_account_retry.client_secret.is_none());
        let service_account_after_interrupted_retry =
            service_account_service::get_service_account(&db, &service_account_id)
                .await
                .expect("reload service account after interrupted retry");
        assert_eq!(
            service_account_after_interrupted_retry.client_secret_hash,
            rotated_service_account_hash,
            "a pending retry must not rotate the secret again"
        );
        let resumed_receipt = receipts
            .find_one(doc! {
                "action": SERVICE_ACCOUNT_ROTATE_SECRET_ACTION,
                "action_request_id": "service-account-rotate-once",
            })
            .await
            .expect("load resumed service-account rotation receipt")
            .expect("resumed service-account rotation receipt exists");
        assert_eq!(
            resumed_receipt.status,
            AssistantActionReceiptStatus::Completed
        );

        let stale_service_account_rotation = rotate_service_account_secret_action(
            State(state.clone()),
            auth(),
            Json(AssistantServiceAccountRotateRequest {
                action_request_id: "service-account-rotate-stale".to_string(),
                service_account_id: service_account_id.clone(),
                expected_updated_at: service_account_expected_at,
            }),
        )
        .await;
        assert!(matches!(
            stale_service_account_rotation,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            service_account_service::get_service_account(&db, &service_account_id)
                .await
                .expect("reload service account after stale fence")
                .client_secret_hash,
            rotated_service_account_hash
        );
        assert_eq!(
            receipts
                .count_documents(doc! {
                    "action_request_id": "service-account-rotate-stale",
                })
                .await
                .expect("count stale rotation receipts"),
            0,
            "a refused optimistic fence must not leave a pending receipt"
        );

        let developer_app_create = || AssistantDeveloperAppCreateRequest {
            action_request_id: "developer-app-create-once".to_string(),
            name: "Assistant receipt application".to_string(),
            redirect_uris: vec!["https://assistant.example/callback".to_string()],
        };
        let created_developer_app =
            create_developer_app_action(State(state.clone()), auth(), Json(developer_app_create()))
                .await
                .expect("create developer app")
                .0;
        assert!(!created_developer_app.replayed);
        assert!(created_developer_app.client_secret.is_some());
        let developer_app_id = created_developer_app.resource.client_id;

        let replayed_developer_app =
            create_developer_app_action(State(state.clone()), auth(), Json(developer_app_create()))
                .await
                .expect("replay developer-app create")
                .0;
        assert!(replayed_developer_app.replayed);
        assert!(replayed_developer_app.client_secret.is_none());
        assert_eq!(
            replayed_developer_app.one_time_material,
            OneTimeMaterialAvailability::Unavailable
        );
        assert_eq!(
            db.collection::<mongodb::bson::Document>(crate::models::oauth_client::COLLECTION_NAME,)
                .count_documents(doc! {
                    "_id": &developer_app_id,
                    "created_by": &actor_id,
                })
                .await
                .expect("count created developer app"),
            1
        );

        let developer_app_before = oauth_client_service::get_client(&db, &developer_app_id)
            .await
            .expect("load developer app before rotation");
        let developer_app_expected_at = developer_app_before.updated_at.to_rfc3339();
        let developer_app_rotate = || AssistantDeveloperAppRotateRequest {
            action_request_id: "developer-app-rotate-once".to_string(),
            client_id: developer_app_id.clone(),
            expected_updated_at: developer_app_expected_at.clone(),
        };
        let rotated_developer_app = rotate_developer_app_secret_action(
            State(state.clone()),
            auth(),
            Json(developer_app_rotate()),
        )
        .await
        .expect("rotate developer-app secret")
        .0;
        assert!(!rotated_developer_app.replayed);
        assert!(rotated_developer_app.client_secret.is_some());
        let developer_app_after = oauth_client_service::get_client(&db, &developer_app_id)
            .await
            .expect("load rotated developer app");
        assert_ne!(
            developer_app_after.client_secret_hash,
            developer_app_before.client_secret_hash
        );

        let replayed_developer_app_rotation = rotate_developer_app_secret_action(
            State(state.clone()),
            auth(),
            Json(developer_app_rotate()),
        )
        .await
        .expect("replay completed developer-app rotation")
        .0;
        assert!(replayed_developer_app_rotation.replayed);
        assert!(replayed_developer_app_rotation.client_secret.is_none());

        let notification_request = || AssistantNotificationsUpdateRequest {
            action_request_id: "notifications-update-interrupted".to_string(),
            telegram_enabled: None,
            approval_required: None,
            approval_timeout_secs: Some(75),
            grant_expiry_days: None,
            push_enabled: None,
        };
        let notification_update =
            update_notifications_action(State(state.clone()), auth(), Json(notification_request()))
                .await
                .expect("update notification settings")
                .0;
        assert!(!notification_update.replayed);
        let notification_before_retry = db
            .collection::<NotificationChannel>(NOTIFICATION_CHANNELS)
            .find_one(doc! { "user_id": &actor_id })
            .await
            .expect("load notification channel")
            .expect("notification channel exists");
        receipts
            .update_one(
                doc! {
                    "user_id": &actor_id,
                    "action": NOTIFICATIONS_UPDATE_ACTION,
                    "action_request_id": "notifications-update-interrupted",
                },
                doc! {
                    "$set": { "status": "pending" },
                    "$unset": { "completed_at": "" },
                },
            )
            .await
            .expect("reopen notification receipt");

        let resumed_notification_update =
            update_notifications_action(State(state), auth(), Json(notification_request()))
                .await
                .expect("resume notification update")
                .0;
        assert!(resumed_notification_update.replayed);
        let notification_after_retry = db
            .collection::<NotificationChannel>(NOTIFICATION_CHANNELS)
            .find_one(doc! { "user_id": &actor_id })
            .await
            .expect("reload notification channel")
            .expect("notification channel exists");
        assert_eq!(notification_after_retry.approval_timeout_secs, 75);
        assert_eq!(
            notification_after_retry.updated_at, notification_before_retry.updated_at,
            "an interrupted retry must not apply the settings update twice"
        );
        let notification_receipt = receipts
            .find_one(doc! {
                "action": NOTIFICATIONS_UPDATE_ACTION,
                "action_request_id": "notifications-update-interrupted",
            })
            .await
            .expect("load resumed notification receipt")
            .expect("notification receipt exists");
        assert_eq!(
            notification_receipt.status,
            AssistantActionReceiptStatus::Completed
        );
    }

    #[tokio::test]
    async fn semantic_field_changes_conflict_across_all_thirty_verbs() {
        // Falsifier: remove any field from a typed fingerprint constructor
        // used by a handler. That field disappears from this exhaustive walk,
        // reducing `changed_field_count`; keeping it but omitting it from the
        // hash makes the same-id reservation below replay instead of conflict.
        use crate::models::service_approval_config::ApprovalVerb;
        use crate::test_utils::connect_test_database;
        use serde_json::Value;

        #[derive(Clone)]
        enum JsonPathSegment {
            Key(String),
            Index(usize),
        }

        fn collect_semantic_leaves(
            value: &Value,
            path: &mut Vec<JsonPathSegment>,
            skip_action: bool,
            leaves: &mut Vec<Vec<JsonPathSegment>>,
        ) {
            match value {
                Value::Object(values) if !values.is_empty() => {
                    for (key, child) in values {
                        if skip_action && key == "action" {
                            continue;
                        }
                        path.push(JsonPathSegment::Key(key.clone()));
                        collect_semantic_leaves(child, path, false, leaves);
                        path.pop();
                    }
                }
                Value::Array(values) if !values.is_empty() => {
                    for (index, child) in values.iter().enumerate() {
                        path.push(JsonPathSegment::Index(index));
                        collect_semantic_leaves(child, path, false, leaves);
                        path.pop();
                    }
                }
                _ => leaves.push(path.clone()),
            }
        }

        fn value_at_path_mut<'a>(
            mut value: &'a mut Value,
            path: &[JsonPathSegment],
        ) -> &'a mut Value {
            for segment in path {
                value = match segment {
                    JsonPathSegment::Key(key) => value
                        .as_object_mut()
                        .expect("path object")
                        .get_mut(key)
                        .expect("path key"),
                    JsonPathSegment::Index(index) => value
                        .as_array_mut()
                        .expect("path array")
                        .get_mut(*index)
                        .expect("path index"),
                };
            }
            value
        }

        fn path_label(path: &[JsonPathSegment]) -> String {
            let mut label = String::new();
            for segment in path {
                match segment {
                    JsonPathSegment::Key(key) => {
                        if !label.is_empty() {
                            label.push('.');
                        }
                        label.push_str(key);
                    }
                    JsonPathSegment::Index(index) => {
                        label.push('[');
                        label.push_str(&index.to_string());
                        label.push(']');
                    }
                }
            }
            label
        }

        let Some(db) = connect_test_database("assistant_org_fingerprint_matrix").await else {
            return;
        };
        let id_a = "00000000-0000-4000-8000-000000000001";
        let id_b = "00000000-0000-4000-8000-000000000002";
        let strings_a = vec!["alpha".to_string()];
        let strings_b = vec!["https://app.example/callback".to_string()];
        let mode = ApprovalMode::PerRequest;
        let default_effect = ApprovalEffect::RequireApproval;
        let rules = vec![ApprovalRule {
            methods: vec!["POST".to_string()],
            resource_pattern: "/v1/*".to_string(),
            verbs: vec![ApprovalVerb::Write],
            effect: ApprovalEffect::RequireApproval,
            mode: ApprovalMode::PerRequest,
        }];
        let mut cases: Vec<(&str, &'static str, String, Value)> = Vec::new();

        macro_rules! fingerprint_case {
            ($name:literal, $action:expr, $value:expr) => {{
                let typed = $value;
                let fingerprint = fingerprint_canonical(&typed).expect("fingerprint case");
                let value = serde_json::to_value(&typed).expect("serialize fingerprint case");
                cases.push(($name, $action, fingerprint, value));
            }};
        }

        fingerprint_case!(
            "account.profile_update",
            ACCOUNT_PROFILE_UPDATE_ACTION,
            ProfileFingerprint {
                action: ACCOUNT_PROFILE_UPDATE_ACTION,
                user_id: id_a,
                display_name: Some("Alice"),
                avatar_url: Some("https://avatar.example/alice.png"),
            }
        );
        fingerprint_case!(
            "account.revoke_consent",
            ACCOUNT_REVOKE_CONSENT_ACTION,
            ConsentFingerprint {
                action: ACCOUNT_REVOKE_CONSENT_ACTION,
                user_id: id_a,
                client_id: id_b,
                confirmed: true,
            }
        );
        fingerprint_case!(
            "account.delete",
            ACCOUNT_DELETE_ACTION,
            ConfirmedAccountDeleteFingerprint {
                action: ACCOUNT_DELETE_ACTION,
                user_id: id_a,
                confirm_email: "owner@example.test",
            }
        );
        fingerprint_case!(
            "account.mfa_setup",
            ACCOUNT_MFA_SETUP_ACTION,
            MfaFingerprint {
                action: ACCOUNT_MFA_SETUP_ACTION,
                user_id: id_a,
                stage: AssistantMfaSetupStage::Confirm,
                factor_id: Some(id_b),
            }
        );
        fingerprint_case!(
            "approval.configure",
            APPROVAL_CONFIGURE_ACTION,
            ApprovalConfigureFingerprint {
                action: APPROVAL_CONFIGURE_ACTION,
                service_id: id_a,
                effective_service_id: id_b,
                approval_required: true,
                approval_mode: &mode,
                rules: &rules,
                default_effect: Some(&default_effect),
            }
        );
        fingerprint_case!(
            "approval.enable",
            APPROVAL_ENABLE_ACTION,
            ApprovalToggleFingerprint {
                action: APPROVAL_ENABLE_ACTION,
                service_id: id_a,
                effective_service_id: id_b,
                approval_required: true,
            }
        );
        fingerprint_case!(
            "approval.disable",
            APPROVAL_DISABLE_ACTION,
            ApprovalToggleFingerprint {
                action: APPROVAL_DISABLE_ACTION,
                service_id: id_a,
                effective_service_id: id_b,
                approval_required: false,
            }
        );
        fingerprint_case!(
            "approval.revoke_grant",
            APPROVAL_REVOKE_GRANT_ACTION,
            GrantFingerprint {
                action: APPROVAL_REVOKE_GRANT_ACTION,
                grant_id: id_a,
                owner_user_id: id_b,
            }
        );

        fingerprint_case!(
            "org.create",
            ORG_CREATE_ACTION,
            Wave4Fingerprint::OrgCreate {
                display_name: "Acme",
                contact_email: Some("owner@acme.test"),
                avatar_url: Some("https://acme.test/avatar.png"),
            }
        );
        fingerprint_case!(
            "org.update",
            ORG_UPDATE_ACTION,
            Wave4Fingerprint::OrgUpdate {
                org_id: id_a,
                display_name: Some("Acme Two"),
                slug: Some("acme-two"),
                contact_email: Some("owner2@acme.test"),
                avatar_url: Some("https://acme.test/avatar-two.png"),
            }
        );
        fingerprint_case!(
            "org.delete",
            ORG_DELETE_ACTION,
            Wave4Fingerprint::OrgDelete {
                org_id: id_a,
                confirmed: true,
            }
        );
        fingerprint_case!(
            "org.member_add",
            ORG_MEMBER_ADD_ACTION,
            Wave4Fingerprint::OrgMemberAdd {
                org_id: id_a,
                user_id: id_b,
                role: orgs::OrgRoleWire::Member,
                allowed_service_ids: Some(&strings_a),
            }
        );
        fingerprint_case!(
            "org.member_remove",
            ORG_MEMBER_REMOVE_ACTION,
            Wave4Fingerprint::OrgMemberRemove {
                org_id: id_a,
                member_id: id_b,
                confirmed: true,
            }
        );
        fingerprint_case!(
            "org.member_update_role",
            ORG_MEMBER_UPDATE_ROLE_ACTION,
            Wave4Fingerprint::OrgMemberUpdateRole {
                org_id: id_a,
                member_id: id_b,
                role: orgs::OrgRoleWire::Admin,
                expected_role: orgs::OrgRoleWire::Member,
            }
        );
        fingerprint_case!(
            "org.invite",
            ORG_INVITE_ACTION,
            Wave4Fingerprint::OrgInvite {
                org_id: id_a,
                role: orgs::OrgRoleWire::Viewer,
                allowed_service_ids: Some(&strings_a),
                ttl_hours: Some(48),
            }
        );
        fingerprint_case!(
            "org.set_primary",
            ORG_SET_PRIMARY_ACTION,
            Wave4Fingerprint::OrgSetPrimary { org_id: id_a }
        );
        fingerprint_case!(
            "service_account.create",
            SERVICE_ACCOUNT_CREATE_ACTION,
            Wave4Fingerprint::ServiceAccountCreate {
                name: "deploy",
                description: Some("Deploy worker"),
                allowed_scopes: Some("proxy"),
                target_org_id: Some(id_a),
            }
        );
        fingerprint_case!(
            "service_account.update",
            SERVICE_ACCOUNT_UPDATE_ACTION,
            Wave4Fingerprint::ServiceAccountUpdate {
                service_account_id: id_a,
                name: Some("deploy two"),
                description: Some("Deploy worker two"),
            }
        );
        fingerprint_case!(
            "service_account.delete",
            SERVICE_ACCOUNT_DELETE_ACTION,
            Wave4Fingerprint::ServiceAccountDelete {
                service_account_id: id_a,
                confirmed: true,
            }
        );
        fingerprint_case!(
            "service_account.rotate_secret",
            SERVICE_ACCOUNT_ROTATE_SECRET_ACTION,
            Wave4Fingerprint::ServiceAccountRotate {
                service_account_id: id_a,
                expected_updated_at: "2026-01-01T00:00:00Z",
            }
        );
        fingerprint_case!(
            "service_account.revoke_tokens",
            SERVICE_ACCOUNT_REVOKE_TOKENS_ACTION,
            Wave4Fingerprint::ServiceAccountRevokeTokens {
                service_account_id: id_a,
                confirmed: true,
            }
        );
        fingerprint_case!(
            "developer_app.create",
            DEVELOPER_APP_CREATE_ACTION,
            Wave4Fingerprint::DeveloperAppCreate {
                name: "Console",
                redirect_uris: &strings_b,
            }
        );
        fingerprint_case!(
            "developer_app.update",
            DEVELOPER_APP_UPDATE_ACTION,
            Wave4Fingerprint::DeveloperAppUpdate {
                client_id: id_a,
                name: Some("Console Two"),
                redirect_uris: Some(&strings_b),
            }
        );
        fingerprint_case!(
            "developer_app.delete",
            DEVELOPER_APP_DELETE_ACTION,
            Wave4Fingerprint::DeveloperAppDelete {
                client_id: id_a,
                confirmed: true,
            }
        );
        fingerprint_case!(
            "developer_app.rotate_secret",
            DEVELOPER_APP_ROTATE_SECRET_ACTION,
            Wave4Fingerprint::DeveloperAppRotate {
                client_id: id_a,
                expected_updated_at: "2026-01-01T00:00:00Z",
            }
        );
        fingerprint_case!(
            "notifications.update",
            NOTIFICATIONS_UPDATE_ACTION,
            Wave4Fingerprint::NotificationsUpdate {
                telegram_enabled: Some(true),
                approval_required: Some(true),
                approval_timeout_secs: Some(60),
                grant_expiry_days: Some(30),
                push_enabled: Some(false),
            }
        );
        fingerprint_case!(
            "notifications.telegram_link",
            NOTIFICATIONS_TELEGRAM_LINK_ACTION,
            Wave4Fingerprint::NotificationsTelegramLink { user_id: id_a }
        );
        fingerprint_case!(
            "notifications.telegram_disconnect",
            NOTIFICATIONS_TELEGRAM_DISCONNECT_ACTION,
            Wave4Fingerprint::NotificationsTelegramDisconnect {
                user_id: id_a,
                confirmed: true,
            }
        );
        fingerprint_case!(
            "external_key.add_gcp_service_account",
            EXTERNAL_KEY_ADD_GCP_ACTION,
            Wave4Fingerprint::ExternalKeyAddGcp {
                label: Some("gcp"),
                key_json_fingerprint: "hmac-sha256:first",
                scopes: Some("scope-a"),
                service_slugs: Some(&strings_a),
                target_org_id: Some(id_a),
            }
        );
        fingerprint_case!(
            "openclaw.connect",
            OPENCLAW_CONNECT_ACTION,
            Wave4Fingerprint::OpenClawConnect {
                gateway_url: "https://gateway.example",
                credential_fingerprint: "hmac-sha256:first",
                label: Some("OpenClaw"),
            }
        );

        assert_eq!(cases.len(), 30, "the table must cover every manifest verb");
        let actor = Uuid::new_v4().to_string();
        let mut changed_field_count = 0usize;
        for (case_index, (name, action, base_fingerprint, serialized)) in
            cases.into_iter().enumerate()
        {
            assert_eq!(serialized["action"].as_str(), Some(action), "{name}");
            let mut leaves = Vec::new();
            let mut path = Vec::new();
            if let Some(request) = serialized.get("request") {
                path.push(JsonPathSegment::Key("request".to_string()));
                collect_semantic_leaves(request, &mut path, false, &mut leaves);
            } else {
                collect_semantic_leaves(&serialized, &mut path, true, &mut leaves);
            }
            assert!(!leaves.is_empty(), "{name} has no bound semantic fields");

            for path in leaves {
                let field = path_label(&path);
                let mut changed = serialized.clone();
                let target = value_at_path_mut(&mut changed, &path);
                match target {
                    Value::Bool(value) => *value = !*value,
                    Value::Number(number) => {
                        let next = number.as_i64().expect("integer field") + 1;
                        *target = Value::from(next);
                    }
                    Value::String(value) => {
                        *value = match value.as_str() {
                            "admin" => "member".to_string(),
                            "member" => "viewer".to_string(),
                            "viewer" => "admin".to_string(),
                            "per_request" => "grant".to_string(),
                            "grant" => "per_request".to_string(),
                            "require_approval" => "auto_allow".to_string(),
                            "auto_allow" => "deny".to_string(),
                            "deny" => "require_approval".to_string(),
                            "confirm" => "start".to_string(),
                            other if other.starts_with("https://") => {
                                "https://different.example/value".to_string()
                            }
                            other => format!("{other}-different"),
                        }
                    }
                    Value::Array(values) => {
                        values.push(Value::String("different".to_string()));
                    }
                    Value::Null => *target = Value::String("present".to_string()),
                    Value::Object(values) => {
                        values.insert("changed".to_string(), Value::Bool(true));
                    }
                }

                let changed_fingerprint =
                    fingerprint_canonical(&changed).expect("changed fingerprint");
                assert_ne!(
                    base_fingerprint, changed_fingerprint,
                    "{name}.{field} is not fingerprinted"
                );
                let action_request_id = format!("fp-{case_index}-{changed_field_count}");
                assert!(matches!(
                    reserve_or_replay(
                        &db,
                        &actor,
                        action,
                        &action_request_id,
                        &base_fingerprint,
                        Uuid::new_v4().to_string(),
                    )
                    .await
                    .expect("reserve base fingerprint"),
                    ReceiptOutcome::Reserved(_)
                ));
                assert!(matches!(
                    reserve_or_replay(
                        &db,
                        &actor,
                        action,
                        &action_request_id,
                        &changed_fingerprint,
                        Uuid::new_v4().to_string(),
                    )
                    .await,
                    Err(AppError::Conflict(_))
                ));
                changed_field_count += 1;
            }
        }
        assert_eq!(
            changed_field_count, 93,
            "adding or removing a bound semantic field requires reviewing this count"
        );
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
        let base = service_account_create_fingerprint("deploy", None, Some("proxy"), None).unwrap();
        let widened =
            service_account_create_fingerprint("deploy", None, Some("proxy admin"), None).unwrap();
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
