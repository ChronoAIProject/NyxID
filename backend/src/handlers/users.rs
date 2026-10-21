use axum::{Json, extract::State, http::HeaderMap};
use chrono::Utc;
use mongodb::bson::{self, doc};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::user::{
    COLLECTION_NAME as USERS, ServiceViewPreferences, ServiceViewSource, ServiceViewState,
    ServiceViewType, User,
};
use crate::mw::auth::AuthUser;
use crate::services::{admin_user_service, role_service};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

// --- Request / Response types ---

/// AI-services (and future) onboarding flags exposed to the frontend.
/// Timestamps are rfc3339 strings; `None` means the flow is not done.
#[derive(Debug, Serialize)]
pub struct OnboardingStateResponse {
    pub ai_services_completed_at: Option<String>,
}

/// User-scoped config / preferences surfaced on `GET /users/me`.
#[derive(Debug, Serialize)]
pub struct ProfileConfigResponse {
    pub services_view: Option<ServiceViewResponse>,
    pub service_views: Option<ServiceViewsResponse>,
    pub onboarding: OnboardingStateResponse,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveServiceViewRequest {
    #[serde(
        default,
        alias = "organization_id",
        deserialize_with = "crate::models::user::deserialize_service_view_ids"
    )]
    pub organization_ids: Vec<String>,
    #[serde(
        default,
        alias = "service_group_id",
        deserialize_with = "crate::models::user::deserialize_service_view_ids"
    )]
    pub service_group_ids: Vec<String>,
    pub search: String,
    pub source: ServiceViewSource,
    pub state: ServiceViewState,
    pub service_type: ServiceViewType,
    pub show_auto_connected: bool,
}

#[derive(Debug, Serialize)]
pub struct ServiceViewResponse {
    pub organization_ids: Vec<String>,
    pub service_group_ids: Vec<String>,
    pub search: String,
    pub source: ServiceViewSource,
    pub state: ServiceViewState,
    pub service_type: ServiceViewType,
    pub show_auto_connected: bool,
}

impl From<ServiceViewPreferences> for ServiceViewResponse {
    fn from(value: ServiceViewPreferences) -> Self {
        Self {
            organization_ids: value.organization_ids,
            service_group_ids: value.service_group_ids,
            search: value.search,
            source: value.source,
            state: value.state,
            service_type: value.service_type,
            show_auto_connected: value.show_auto_connected,
        }
    }
}

/// PUT /api/v1/users/me/preferences/services
pub async fn save_services_view(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<SaveServiceViewRequest>,
) -> AppResult<Json<ServiceViewResponse>> {
    let preferences = body.into();
    let saved = crate::services::user_preferences_service::save_services_view(
        &state.db,
        &auth_user.user_id.to_string(),
        preferences,
    )
    .await?;
    Ok(Json(saved.into()))
}

impl From<SaveServiceViewRequest> for ServiceViewPreferences {
    fn from(body: SaveServiceViewRequest) -> Self {
        Self {
            organization_ids: body.organization_ids,
            service_group_ids: body.service_group_ids,
            search: body.search,
            source: body.source,
            state: body.state,
            service_type: body.service_type,
            show_auto_connected: body.show_auto_connected,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedServiceViewRequest {
    pub id: String,
    pub name: String,
    pub filters: SaveServiceViewRequest,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveServiceViewsRequest {
    pub views: Vec<SavedServiceViewRequest>,
    pub default_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SavedServiceViewResponse {
    pub id: String,
    pub name: String,
    pub filters: ServiceViewResponse,
}

#[derive(Debug, Serialize)]
pub struct ServiceViewsResponse {
    pub views: Vec<SavedServiceViewResponse>,
    pub default_id: Option<String>,
}

impl From<crate::models::user::ServiceViewsPreferences> for ServiceViewsResponse {
    fn from(value: crate::models::user::ServiceViewsPreferences) -> Self {
        Self {
            default_id: value.default_id,
            views: value
                .views
                .into_iter()
                .map(|view| SavedServiceViewResponse {
                    id: view.id,
                    name: view.name,
                    filters: view.filters.into(),
                })
                .collect(),
        }
    }
}

pub async fn save_service_views(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<SaveServiceViewsRequest>,
) -> AppResult<Json<ServiceViewsResponse>> {
    let preferences = crate::models::user::ServiceViewsPreferences {
        default_id: body.default_id,
        views: body
            .views
            .into_iter()
            .map(|view| crate::models::user::SavedServiceView {
                id: view.id,
                name: view.name,
                filters: view.filters.into(),
            })
            .collect(),
    };
    Ok(Json(
        crate::services::user_preferences_service::save_service_views(
            &state.db,
            &auth_user.user_id.to_string(),
            preferences,
        )
        .await?
        .into(),
    ))
}

#[derive(Debug, Serialize)]
pub struct UserCapabilitiesResponse {
    pub billing_available: bool,
    /// Whether the caller may issue and revoke one-off credit grants
    /// (Admin, or Operator with the Credits Manager role).
    pub manage_credit_grants: bool,
    /// Whether the caller may create providers and catalog services
    /// (Admin, or Operator with the Catalog Manager role).
    pub manage_catalog: bool,
    pub manage_tools: bool,
    /// Whether the caller may read platform-wide admin usage (Admin,
    /// Operator, or a role carrying `nyxid:admin:usage:read`).
    pub read_admin_usage: bool,
    pub read_feature_flags: bool,
    pub create_feature_flags: bool,
    pub manage_all_feature_flags: bool,
    /// Feature-flag keys enabled for this user on personal (non-org)
    /// surfaces, resolved server-side with specificity precedence:
    /// default -> global -> matching org -> user. Org-scoped pages
    /// still use `OrgResponse.enabled_features` for that org's own context.
    pub enabled_features: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct UserProfileResponse {
    /// Authoritative subject type for downstream organization ownership.
    pub user_type: crate::models::user::UserType,
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub email_verified: bool,
    pub mfa_enabled: bool,
    pub is_admin: bool,
    pub is_operator: bool,
    /// Resolved platform role: `"admin"`, `"operator"`, or `"user"`.
    pub role: String,
    pub is_active: bool,
    pub social_provider: Option<String>,
    pub has_password: bool,
    pub created_at: String,
    pub updated_at: String,
    pub last_login_at: Option<String>,
    pub profile_config: ProfileConfigResponse,
    pub capabilities: UserCapabilitiesResponse,
}

/// Secret-free, free-text-free account state for assistant postconditions.
///
/// The full profile response contains user-controlled display text and is not
/// safe for recursive evidence scanning. This projection is derived from that
/// response and deliberately exposes only primitive state.
#[derive(Debug, Serialize)]
pub struct AccountAuthorizationEvidenceResponse {
    pub id: String,
    pub is_active: bool,
    pub mfa_enabled: bool,
    pub display_name_length: Option<usize>,
    pub avatar_url_length: Option<usize>,
    pub updated_at: String,
}

fn javascript_string_length(value: &str) -> usize {
    value.encode_utf16().count()
}

impl AccountAuthorizationEvidenceResponse {
    fn from_profile_response(profile: &UserProfileResponse) -> Self {
        Self {
            id: profile.id.clone(),
            is_active: profile.is_active,
            mfa_enabled: profile.mfa_enabled,
            display_name_length: profile
                .display_name
                .as_deref()
                .map(javascript_string_length),
            avatar_url_length: profile.avatar_url.as_deref().map(javascript_string_length),
            updated_at: profile.updated_at.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UpdateProfileResponse {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub message: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DeleteAccountResponse {
    pub job_id: String,
    pub status: crate::models::personal_erasure::ErasureStatus,
    pub accepted_at: String,
    pub completed_at: Option<String>,
    pub domains: Vec<ErasureDomainResponse>,
    pub effects: Vec<super::admin::ErasureEffectResponse>,
    pub next_effect_after: Option<String>,
}
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ErasureDomainResponse {
    pub retry_generation: i64,
    pub domain: String,
    pub status: crate::models::personal_erasure::DomainStatus,
    pub reason: Option<String>,
    pub completed_at: Option<String>,
}
impl DeleteAccountResponse {
    pub(crate) async fn from_job(
        db: &mongodb::Database,
        job: crate::models::personal_erasure::PersonalErasureJob,
    ) -> AppResult<Self> {
        let effects =
            crate::services::personal_erasure_service::issuance::outcomes(db, &job.user_id, None)
                .await?;
        let next_effect_after = (effects.len() == 64).then(|| effects.last().unwrap().id.clone());
        Ok(Self {
            effects: effects
                .into_iter()
                .map(super::admin::ErasureEffectResponse::from)
                .collect(),
            next_effect_after,
            job_id: job.id,
            status: job.status,
            accepted_at: job.accepted_at.to_rfc3339(),
            completed_at: job.completed_at.map(|v| v.to_rfc3339()),
            domains: job
                .domains
                .into_iter()
                .map(|d| ErasureDomainResponse {
                    retry_generation: d.retry_generation,
                    domain: d.domain,
                    status: d.status,
                    reason: d.reason,
                    completed_at: d.completed_at.map(|v| v.to_rfc3339()),
                })
                .collect(),
        })
    }
}

// --- Handlers ---

/// GET /api/v1/users/me
///
/// Returns the profile of the currently authenticated user.
pub async fn get_me(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<UserProfileResponse>> {
    let user_id = auth_user.user_id.to_string();

    let user_model = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let platform_role = role_service::resolve_platform_role(&state.db, &user_model).await?;
    let role = platform_role.as_str().to_string();
    let (is_admin, is_operator) = platform_role.legacy_flags();
    let manage_credit_grants =
        role_service::can_manage_credit_grants(&state.db, &user_model, platform_role).await?;
    let manage_tools =
        role_service::can_manage_tools(&state.db, &user_model, platform_role).await?;
    let manage_catalog =
        role_service::can_manage_catalog(&state.db, &user_model, platform_role).await?;
    let read_admin_usage =
        role_service::can_read_admin_usage(&state.db, &user_model, platform_role).await?;
    let feature_flags =
        role_service::feature_flag_access(&state.db, &user_model, platform_role).await?;
    // Personal-surface feature-flag resolution: org-aware specificity, so an
    // org override can override the global baseline for its members
    // (sidebar, /assistant guard).
    let enabled_features =
        crate::services::feature_flag_service::resolve_personal_features(&state.db, &user_id)
            .await?;
    Ok(Json(UserProfileResponse {
        user_type: user_model.user_type,
        id: user_model.id,
        email: user_model.email,
        display_name: user_model.display_name,
        avatar_url: user_model.avatar_url,
        email_verified: user_model.email_verified,
        mfa_enabled: user_model.mfa_enabled,
        is_admin,
        is_operator,
        role,
        is_active: user_model.is_active,
        has_password: user_model.password_hash.is_some(),
        social_provider: user_model.social_provider,
        created_at: user_model.created_at.to_rfc3339(),
        updated_at: user_model.updated_at.to_rfc3339(),
        last_login_at: user_model.last_login_at.map(|t| t.to_rfc3339()),
        profile_config: ProfileConfigResponse {
            services_view: user_model.profile_config.services_view.map(Into::into),
            service_views: user_model.profile_config.service_views.map(Into::into),
            onboarding: OnboardingStateResponse {
                ai_services_completed_at: user_model
                    .profile_config
                    .onboarding
                    .ai_services_completed_at
                    .map(|t| t.to_rfc3339()),
            },
        },
        capabilities: UserCapabilitiesResponse {
            billing_available: state.billing.billing_enabled()
                && state.billing.lago_configured()
                && enabled_features
                    .iter()
                    .any(|key| key == crate::services::feature_flag_service::BILLING_FLAG_KEY),
            manage_credit_grants,
            manage_catalog,
            manage_tools,
            read_admin_usage,
            read_feature_flags: feature_flags.read_all || feature_flags.manage_own,
            create_feature_flags: feature_flags.manage_own,
            manage_all_feature_flags: feature_flags.manage_all,
            enabled_features,
        },
    }))
}

/// GET /api/v1/users/me/authorization
///
/// Authoritative account postcondition projection. This must be mounted next
/// to `GET /users/me` in both the normal and delegated `account:read` route
/// inventories; it must never be served from the assistant-effect nest.
pub async fn get_account_authorization(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<AccountAuthorizationEvidenceResponse>> {
    let Json(profile) = get_me(State(state), auth_user).await?;
    Ok(Json(
        AccountAuthorizationEvidenceResponse::from_profile_response(&profile),
    ))
}

#[derive(Debug, Deserialize)]
pub struct CompleteOnboardingRequest {
    /// Onboarding flow identifier. Currently only `"ai_services"`.
    pub key: String,
}

/// POST /api/v1/users/me/onboarding/complete
///
/// Marks a first-run onboarding flow as completed (or skipped) for the
/// caller, so the post-login wizard redirect stops firing. Idempotent:
/// re-stamping an already-completed flow just refreshes the timestamp.
pub async fn complete_onboarding(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<CompleteOnboardingRequest>,
) -> AppResult<Json<OnboardingStateResponse>> {
    let user_id = auth_user.user_id.to_string();

    let field = match req.key.as_str() {
        "ai_services" => "profile_config.onboarding.ai_services_completed_at",
        other => {
            return Err(AppError::BadRequest(format!(
                "Unknown onboarding key: {other}"
            )));
        }
    };

    let now = Utc::now();
    // Build the `$set` doc explicitly: `field` is a dynamic dotted path, not
    // a literal, so insert it rather than relying on `doc!` key parsing.
    let mut set_doc = bson::Document::new();
    set_doc.insert(field, bson::DateTime::from_chrono(now));
    let result = state
        .db
        .collection::<User>(USERS)
        .update_one(doc! { "_id": &user_id }, doc! { "$set": set_doc })
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("User not found".to_string()));
    }

    Ok(Json(OnboardingStateResponse {
        ai_services_completed_at: Some(now.to_rfc3339()),
    }))
}

/// PUT /api/v1/users/me
///
/// Update the profile of the currently authenticated user.
pub async fn update_me(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<UpdateProfileRequest>,
) -> AppResult<Json<UpdateProfileResponse>> {
    let user_id = auth_user.user_id.to_string();

    // Verify user exists
    let _existing = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let mut set_doc = doc! {};

    if let Some(ref name) = body.display_name {
        if name.len() > 200 {
            return Err(AppError::ValidationError(
                "Display name must be 200 characters or less".to_string(),
            ));
        }
        set_doc.insert("display_name", name);
    }

    if let Some(ref url) = body.avatar_url {
        if url.len() > 2048 {
            return Err(AppError::ValidationError(
                "Avatar URL must be 2048 characters or less".to_string(),
            ));
        }
        // Validate URL scheme to prevent javascript: and data: URI injection
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return Err(AppError::ValidationError(
                "Avatar URL must use https:// or http:// scheme".to_string(),
            ));
        }
        set_doc.insert("avatar_url", url);
    }

    let now = chrono::Utc::now();
    set_doc.insert("updated_at", bson::DateTime::from_chrono(now));

    state
        .db
        .collection::<User>(USERS)
        .update_one(doc! { "_id": &user_id }, doc! { "$set": set_doc })
        .await?;

    // Re-fetch the updated user
    let updated = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::Internal("User disappeared after update".to_string()))?;

    Ok(Json(UpdateProfileResponse {
        id: updated.id,
        email: updated.email,
        display_name: updated.display_name,
        avatar_url: updated.avatar_url,
        message: "Profile updated successfully".to_string(),
    }))
}

/// DELETE /api/v1/users/me
///
/// Permanently delete the currently authenticated user and related credentials/sessions.
pub async fn delete_me(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    headers: HeaderMap,
) -> AppResult<Json<DeleteAccountResponse>> {
    super::login_client_context::require_first_party_human(&auth_user)?;
    let deletion_policy = headers
        .get(admin_user_service::ACCOUNT_DELETION_POLICY_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    admin_user_service::validate_account_deletion_policy(deletion_policy)?;
    let user_id = auth_user.user_id.to_string();

    let job = crate::services::personal_erasure_service::accept(
        &state.db,
        &user_id,
        &user_id,
        Some(deletion_policy),
    )
    .await?;
    // Acceptance closes authority immediately. Remote/local work continues
    // durably; analytics must not describe this transition as completed.
    let _ = tele;
    Ok(Json(DeleteAccountResponse::from_job(&state.db, job).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::UserType;
    use crate::services::role_service;
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};
    use uuid::Uuid;

    fn service_view_request() -> SaveServiceViewRequest {
        SaveServiceViewRequest {
            organization_ids: vec!["org-selection".into(), "org-second".into()],
            service_group_ids: vec!["catalog:service-selection".into(), "catalog:second".into()],
            search: "team".into(),
            source: ServiceViewSource::Org,
            state: ServiceViewState::Enabled,
            service_type: ServiceViewType::Http,
            show_auto_connected: false,
        }
    }

    #[tokio::test]
    async fn named_service_views_round_trip_and_clear_deleted_default() {
        let db = connect_test_database("named_service_views").await.unwrap();
        let user_id = Uuid::new_v4().to_string();
        let other_id = Uuid::new_v4().to_string();
        let mut user = test_user(&user_id, UserType::Person);
        user.profile_config.onboarding.ai_services_completed_at =
            Some(bson::DateTime::now().to_chrono());
        db.collection::<User>(USERS)
            .insert_many([user.clone(), test_user(&other_id, UserType::Person)])
            .await
            .unwrap();
        let state = test_app_state(db.clone());
        let response = save_service_views(
            State(state.clone()),
            test_auth_user(&user_id),
            Json(SaveServiceViewsRequest {
                views: vec![
                    SavedServiceViewRequest {
                        id: "first".into(),
                        name: " First ".into(),
                        filters: service_view_request(),
                    },
                    SavedServiceViewRequest {
                        id: "second".into(),
                        name: "Second".into(),
                        filters: service_view_request(),
                    },
                ],
                default_id: Some("second".into()),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(response.views.len(), 2);
        assert_eq!(response.views[0].name, "First");
        let persisted = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &user_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            persisted
                .profile_config
                .service_views
                .unwrap()
                .default_id
                .as_deref(),
            Some("second")
        );
        assert_eq!(
            persisted.profile_config.services_view.unwrap().search,
            "team"
        );
        assert_eq!(
            persisted.profile_config.onboarding,
            user.profile_config.onboarding
        );
        let other = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &other_id })
            .await
            .unwrap()
            .unwrap();
        assert!(other.profile_config.service_views.is_none());
        let mut legacy_update = service_view_request();
        legacy_update.search = "$literal search".into();
        let legacy_response = save_services_view(
            State(state.clone()),
            test_auth_user(&user_id),
            Json(legacy_update),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(legacy_response.search, "$literal search");
        let updated = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &user_id })
            .await
            .unwrap()
            .unwrap();
        let views = updated.profile_config.service_views.unwrap().views;
        assert_eq!(views[0].filters.search, "team");
        assert_eq!(views[1].filters.search, "$literal search");
        assert_eq!(
            updated.profile_config.services_view.unwrap().search,
            "$literal search"
        );
        let deleted = save_service_views(
            State(state),
            test_auth_user(&user_id),
            Json(SaveServiceViewsRequest {
                views: vec![SavedServiceViewRequest {
                    id: "first".into(),
                    name: "First".into(),
                    filters: service_view_request(),
                }],
                default_id: None,
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(deleted.default_id.is_none());
        assert_eq!(deleted.views.len(), 1);
        let persisted = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &user_id })
            .await
            .unwrap()
            .unwrap();
        assert!(persisted.profile_config.services_view.is_none());
        assert_eq!(
            persisted.profile_config.service_views.unwrap().views.len(),
            1
        );
    }

    #[tokio::test]
    async fn services_view_round_trips_without_overwriting_other_settings_or_users() {
        let db = connect_test_database("services_view_round_trip")
            .await
            .unwrap();
        let user_id = Uuid::new_v4().to_string();
        let other_id = Uuid::new_v4().to_string();
        let mut user = test_user(&user_id, UserType::Person);
        user.profile_config.onboarding.ai_services_completed_at =
            chrono::DateTime::from_timestamp_millis(Utc::now().timestamp_millis());
        user.profile_config
            .release_integrity
            .remote_credential_integrity_verification_opt_out = true;
        db.collection::<User>(USERS)
            .insert_many([user.clone(), test_user(&other_id, UserType::Person)])
            .await
            .unwrap();
        db.collection::<User>(USERS)
            .update_one(
                doc! { "_id": &user_id },
                doc! { "$set": { "profile_config.future_setting": "preserve" } },
            )
            .await
            .unwrap();
        role_service::seed_system_roles(&db).await.unwrap();
        let state = test_app_state(db.clone());
        let before = get_me(State(state.clone()), test_auth_user(&user_id))
            .await
            .unwrap()
            .0;
        assert!(before.profile_config.services_view.is_none());
        let saved = save_services_view(
            State(state.clone()),
            test_auth_user(&user_id),
            Json(service_view_request()),
        )
        .await
        .unwrap()
        .0;
        let loaded = get_me(State(state.clone()), test_auth_user(&user_id))
            .await
            .unwrap()
            .0;
        assert_eq!(
            serde_json::to_value(&saved).unwrap(),
            serde_json::to_value(loaded.profile_config.services_view.unwrap()).unwrap()
        );
        let persisted = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &user_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            persisted.profile_config.onboarding,
            user.profile_config.onboarding
        );
        assert_eq!(
            persisted.profile_config.release_integrity,
            user.profile_config.release_integrity
        );
        let raw = db
            .collection::<bson::Document>(USERS)
            .find_one(doc! { "_id": &user_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            raw.get_document("profile_config")
                .unwrap()
                .get_str("future_setting")
                .unwrap(),
            "preserve"
        );
        let other = get_me(State(state.clone()), test_auth_user(&other_id))
            .await
            .unwrap()
            .0;
        assert!(other.profile_config.services_view.is_none());

        let mut clear = service_view_request();
        clear.search.clear();
        clear.organization_ids.clear();
        clear.service_group_ids.clear();
        clear.source = ServiceViewSource::All;
        clear.state = ServiceViewState::All;
        clear.service_type = ServiceViewType::All;
        clear.show_auto_connected = true;
        let _ = save_services_view(State(state.clone()), test_auth_user(&user_id), Json(clear))
            .await
            .unwrap();
        let reset = get_me(State(state), test_auth_user(&user_id))
            .await
            .unwrap()
            .0;
        assert_eq!(
            reset.profile_config.services_view.unwrap().source,
            ServiceViewSource::All
        );
    }

    #[tokio::test]
    async fn services_view_validates_and_supports_legacy_profiles() {
        let db = connect_test_database("services_view_legacy").await.unwrap();
        let user_id = Uuid::new_v4().to_string();
        let mut raw = bson::to_document(&test_user(&user_id, UserType::Person)).unwrap();
        raw.remove("profile_config");
        db.collection::<bson::Document>(USERS)
            .insert_one(raw)
            .await
            .unwrap();
        let state = test_app_state(db.clone());
        let mut invalid = service_view_request();
        invalid.search = "x".repeat(201);
        assert!(matches!(
            save_services_view(
                State(state.clone()),
                test_auth_user(&user_id),
                Json(invalid)
            )
            .await,
            Err(AppError::ValidationError(_))
        ));
        for (organizations, services) in [
            (vec!["org".to_string(); 101], vec![]),
            (vec![], vec![" ".to_string()]),
            (vec![], vec!["x".repeat(129)]),
        ] {
            let mut invalid = service_view_request();
            invalid.organization_ids = organizations;
            invalid.service_group_ids = services;
            assert!(matches!(
                save_services_view(
                    State(state.clone()),
                    test_auth_user(&user_id),
                    Json(invalid)
                )
                .await,
                Err(AppError::ValidationError(_))
            ));
        }
        let unchanged = db
            .collection::<bson::Document>(USERS)
            .find_one(doc! { "_id": &user_id })
            .await
            .unwrap()
            .unwrap();
        assert!(!unchanged.contains_key("profile_config"));
        let _ = save_services_view(
            State(state.clone()),
            test_auth_user(&user_id),
            Json(service_view_request()),
        )
        .await
        .unwrap();
        let loaded = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &user_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.profile_config.services_view.unwrap().search, "team");
        assert!(matches!(
            save_services_view(
                State(state),
                test_auth_user(&Uuid::new_v4().to_string()),
                Json(service_view_request())
            )
            .await,
            Err(AppError::NotFound(_))
        ));
    }

    #[test]
    fn services_view_reads_legacy_selections_and_writes_arrays() {
        let legacy = serde_json::json!({ "search": "", "organization_id": "org-1", "service_group_id": null,
            "source": "all", "state": "all", "service_type": "all", "show_auto_connected": true });
        let persisted: ServiceViewPreferences = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(persisted.organization_ids, vec!["org-1"]);
        assert!(persisted.service_group_ids.is_empty());
        let request: SaveServiceViewRequest = serde_json::from_value(legacy).unwrap();
        assert_eq!(request.organization_ids, vec!["org-1"]);
        let response = serde_json::to_value(ServiceViewResponse::from(persisted)).unwrap();
        assert_eq!(response["organization_ids"], serde_json::json!(["org-1"]));
        assert_eq!(response["service_group_ids"], serde_json::json!([]));
        assert!(response.get("organization_id").is_none());
    }

    #[test]
    fn services_view_rejects_unknown_filter_values_and_owner_injection() {
        let body = serde_json::json!({ "search": "", "source": "all", "state": "all",
            "service_type": "all", "show_auto_connected": true });
        for (field, value) in [
            ("source", "admin"),
            ("state", "healthy"),
            ("service_type", "ftp"),
            ("user_id", "someone-else"),
        ] {
            let mut invalid = body.clone();
            invalid[field] = value.into();
            assert!(serde_json::from_value::<SaveServiceViewRequest>(invalid).is_err());
        }
        assert!(serde_json::from_value::<SaveServiceViewRequest>(body).is_ok());
    }

    #[tokio::test]
    async fn get_me_derives_platform_role_fields_from_rbac_membership() {
        let Some(db) = connect_test_database("users_me_platform_role").await else {
            eprintln!("skipping users/me role test: no local MongoDB available");
            return;
        };
        role_service::seed_system_roles(&db)
            .await
            .expect("seed platform roles");
        let platform_role_ids = role_service::get_platform_role_ids(&db)
            .await
            .expect("platform role ids");

        let user_id = Uuid::new_v4().to_string();
        let mut user = test_user(&user_id, UserType::Person);
        user.role_ids.push(platform_role_ids.operator);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .expect("insert user");
        let state = test_app_state(db);

        let response = get_me(State(state), test_auth_user(&user_id))
            .await
            .expect("get profile");

        assert_eq!(response.0.role, "operator");
        assert!(!response.0.is_admin);
        assert!(response.0.is_operator);
    }

    #[tokio::test]
    async fn get_me_marks_billing_unavailable_without_lago() {
        let Some(db) = connect_test_database("users_me_billing_unavailable").await else {
            eprintln!("skipping users/me billing capability test: no local MongoDB available");
            return;
        };

        let user_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .expect("insert user");
        role_service::seed_system_roles(&db)
            .await
            .expect("seed platform roles");

        let response = get_me(State(test_app_state(db)), test_auth_user(&user_id))
            .await
            .expect("get profile");

        assert!(!response.0.capabilities.billing_available);
    }

    #[tokio::test]
    async fn get_me_billing_follows_org_and_user_precedence() {
        use crate::handlers::billing::ensure_billing_rollout;
        use crate::models::org_membership::{
            COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole,
        };
        use crate::services::feature_flag_service::{self as flags, BILLING_FLAG_KEY, FlagTarget};
        use crate::test_utils::{test_app_config, test_app_state_with_config, test_membership};

        let db = connect_test_database("users_me_billing_precedence")
            .await
            .unwrap();
        role_service::seed_system_roles(&db).await.unwrap();
        let user_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&user_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .unwrap();
        let membership = test_membership(&org_id, &user_id, OrgRole::Member, None);
        db.collection::<OrgMembership>(MEMBERSHIPS)
            .insert_one(&membership)
            .await
            .unwrap();
        let lago = wiremock::MockServer::start().await;
        let mut config = test_app_config();
        config.billing_enabled = true;
        config.lago_api_url = Some(lago.uri());
        config.lago_api_key = Some("test-key".to_string());
        let state = test_app_state_with_config(db.clone(), config);
        flags::set_platform_override(&db, BILLING_FLAG_KEY, &FlagTarget::Global, true, &user_id)
            .await
            .unwrap();

        for (case, org_value, user_value, active_member, expected) in [
            (
                "org disable beats global enable",
                Some(false),
                None,
                true,
                false,
            ),
            (
                "user enable beats org disable",
                Some(false),
                Some(true),
                true,
                true,
            ),
            (
                "cleared user inherits org disable",
                Some(false),
                None,
                true,
                false,
            ),
            ("cleared org inherits global enable", None, None, true, true),
            (
                "user disable beats org enable",
                Some(true),
                Some(false),
                true,
                false,
            ),
            (
                "revoked membership ignores org disable",
                Some(false),
                None,
                false,
                true,
            ),
        ] {
            if let Some(value) = org_value {
                flags::set_platform_org_override(&db, &org_id, BILLING_FLAG_KEY, value, &user_id)
                    .await
                    .unwrap();
            } else {
                flags::clear_platform_org_override(&db, &org_id, BILLING_FLAG_KEY)
                    .await
                    .unwrap();
            }
            let user_target = FlagTarget::User(user_id.clone());
            if let Some(value) = user_value {
                flags::set_platform_override(&db, BILLING_FLAG_KEY, &user_target, value, &user_id)
                    .await
                    .unwrap();
            } else {
                flags::clear_platform_override(&db, BILLING_FLAG_KEY, &user_target)
                    .await
                    .unwrap();
            }
            let revoked_at = if active_member {
                bson::Bson::Null
            } else {
                bson::Bson::DateTime(bson::DateTime::now())
            };
            db.collection::<OrgMembership>(MEMBERSHIPS)
                .update_one(
                    doc! { "_id": &membership.id },
                    doc! { "$set": { "revoked_at": revoked_at } },
                )
                .await
                .unwrap();

            let response = get_me(State(state.clone()), test_auth_user(&user_id))
                .await
                .unwrap()
                .0;
            assert_eq!(response.capabilities.billing_available, expected, "{case}");
            assert_eq!(
                response
                    .capabilities
                    .enabled_features
                    .iter()
                    .any(|key| key == BILLING_FLAG_KEY),
                expected,
                "{case}"
            );
            let gate = ensure_billing_rollout(&state, &user_id, &user_id).await;
            if expected {
                gate.expect(case);
            } else {
                assert!(matches!(gate, Err(AppError::Forbidden(_))), "{case}");
            }
            if active_member {
                assert_eq!(
                    flags::billing_rollout_enabled(&db, &org_id, &user_id)
                        .await
                        .unwrap(),
                    expected,
                    "{case}"
                );
            }
        }
        db.drop().await.unwrap();
    }

    // --- Serialization tests: UserProfileResponse ---

    #[test]
    fn user_profile_response_serialization_full() {
        let resp = UserProfileResponse {
            user_type: crate::models::user::UserType::Person,
            id: "user-1".to_string(),
            email: "test@example.com".to_string(),
            display_name: Some("Alice".to_string()),
            avatar_url: Some("https://example.com/avatar.png".to_string()),
            email_verified: true,
            mfa_enabled: false,
            is_admin: true,
            is_operator: false,
            role: "admin".to_string(),
            is_active: true,
            social_provider: Some("google".to_string()),
            has_password: false,
            created_at: "2025-01-01T00:00:00+00:00".to_string(),
            updated_at: "2025-01-01T00:00:00+00:00".to_string(),
            last_login_at: Some("2025-06-01T12:00:00+00:00".to_string()),
            profile_config: ProfileConfigResponse {
                services_view: None,
                service_views: None,
                onboarding: OnboardingStateResponse {
                    ai_services_completed_at: Some("2025-03-15T10:00:00+00:00".to_string()),
                },
            },
            capabilities: UserCapabilitiesResponse {
                billing_available: true,
                manage_credit_grants: false,
                manage_catalog: false,
                manage_tools: false,
                read_admin_usage: false,
                read_feature_flags: false,
                create_feature_flags: false,
                manage_all_feature_flags: false,
                enabled_features: vec![],
            },
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["id"], "user-1");
        assert_eq!(json["email"], "test@example.com");
        assert_eq!(json["display_name"], "Alice");
        assert_eq!(json["avatar_url"], "https://example.com/avatar.png");
        assert_eq!(json["email_verified"], true);
        assert_eq!(json["mfa_enabled"], false);
        assert_eq!(json["is_admin"], true);
        assert_eq!(json["is_operator"], false);
        assert_eq!(json["role"], "admin");
        assert_eq!(json["is_active"], true);
        assert_eq!(json["social_provider"], "google");
        assert_eq!(json["created_at"], "2025-01-01T00:00:00+00:00");
        assert_eq!(json["last_login_at"], "2025-06-01T12:00:00+00:00");
        assert_eq!(
            json["profile_config"]["onboarding"]["ai_services_completed_at"],
            "2025-03-15T10:00:00+00:00"
        );
        assert_eq!(json["capabilities"]["billing_available"], true);
    }

    #[test]
    fn user_profile_response_serialization_minimal() {
        let resp = UserProfileResponse {
            user_type: crate::models::user::UserType::Person,
            id: "user-2".to_string(),
            email: "minimal@example.com".to_string(),
            display_name: None,
            avatar_url: None,
            email_verified: false,
            mfa_enabled: false,
            is_admin: false,
            is_operator: false,
            role: "user".to_string(),
            is_active: true,
            social_provider: None,
            has_password: false,
            created_at: "2025-06-01T00:00:00+00:00".to_string(),
            updated_at: "2025-06-01T00:00:00+00:00".to_string(),
            last_login_at: None,
            profile_config: ProfileConfigResponse {
                services_view: None,
                service_views: None,
                onboarding: OnboardingStateResponse {
                    ai_services_completed_at: None,
                },
            },
            capabilities: UserCapabilitiesResponse {
                billing_available: false,
                manage_credit_grants: false,
                manage_catalog: false,
                manage_tools: false,
                read_admin_usage: false,
                read_feature_flags: false,
                create_feature_flags: false,
                manage_all_feature_flags: false,
                enabled_features: vec![],
            },
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert!(json["display_name"].is_null());
        assert!(json["avatar_url"].is_null());
        assert!(json["social_provider"].is_null());
        assert!(json["last_login_at"].is_null());
        assert!(json["profile_config"]["onboarding"]["ai_services_completed_at"].is_null());
        assert_eq!(json["capabilities"]["billing_available"], false);
        assert_eq!(json["role"], "user");
    }

    #[test]
    fn profile_update_verification_accepts_multibyte_display_name() {
        // Falsifier: replace `javascript_string_length` with `str::len` and
        // this reports UTF-8 bytes (10) instead of JavaScript code units (7).
        let profile = UserProfileResponse {
            user_type: crate::models::user::UserType::Person,
            id: "user-multibyte".to_string(),
            email: "multibyte@example.com".to_string(),
            display_name: Some("café 🧭".to_string()),
            avatar_url: None,
            email_verified: true,
            mfa_enabled: false,
            is_admin: false,
            is_operator: false,
            role: "user".to_string(),
            is_active: true,
            social_provider: None,
            has_password: false,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:01Z".to_string(),
            last_login_at: None,
            profile_config: ProfileConfigResponse {
                services_view: None,
                service_views: None,
                onboarding: OnboardingStateResponse {
                    ai_services_completed_at: None,
                },
            },
            capabilities: UserCapabilitiesResponse {
                billing_available: false,
                manage_credit_grants: false,
                manage_catalog: false,
                manage_tools: false,
                read_admin_usage: false,
                read_feature_flags: false,
                create_feature_flags: false,
                manage_all_feature_flags: false,
                enabled_features: vec![],
            },
        };

        let evidence = AccountAuthorizationEvidenceResponse::from_profile_response(&profile);
        assert_eq!(evidence.display_name_length, Some(7));
        assert_ne!(evidence.display_name_length, Some("café 🧭".len()));
    }

    // --- Serialization tests: OnboardingStateResponse ---

    #[test]
    fn onboarding_state_response_serialization_with_timestamp() {
        let resp = OnboardingStateResponse {
            ai_services_completed_at: Some("2025-05-20T08:30:00+00:00".to_string()),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(
            json["ai_services_completed_at"],
            "2025-05-20T08:30:00+00:00"
        );
    }

    #[test]
    fn onboarding_state_response_serialization_without_timestamp() {
        let resp = OnboardingStateResponse {
            ai_services_completed_at: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json["ai_services_completed_at"].is_null());
    }

    // --- Serialization tests: ProfileConfigResponse ---

    #[test]
    fn profile_config_response_serialization() {
        let resp = ProfileConfigResponse {
            services_view: None,
            service_views: None,
            onboarding: OnboardingStateResponse {
                ai_services_completed_at: Some("2025-01-01T00:00:00+00:00".to_string()),
            },
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json["onboarding"].is_object());
        assert_eq!(
            json["onboarding"]["ai_services_completed_at"],
            "2025-01-01T00:00:00+00:00"
        );
    }

    // --- Serialization tests: UpdateProfileResponse ---

    #[test]
    fn update_profile_response_serialization_full() {
        let resp = UpdateProfileResponse {
            id: "user-1".to_string(),
            email: "updated@example.com".to_string(),
            display_name: Some("Bob".to_string()),
            avatar_url: Some("https://cdn.example.com/bob.jpg".to_string()),
            message: "Profile updated successfully".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["id"], "user-1");
        assert_eq!(json["email"], "updated@example.com");
        assert_eq!(json["display_name"], "Bob");
        assert_eq!(json["avatar_url"], "https://cdn.example.com/bob.jpg");
        assert_eq!(json["message"], "Profile updated successfully");
    }

    #[test]
    fn update_profile_response_serialization_with_nulls() {
        let resp = UpdateProfileResponse {
            id: "user-2".to_string(),
            email: "test@example.com".to_string(),
            display_name: None,
            avatar_url: None,
            message: "Profile updated successfully".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert!(json["display_name"].is_null());
        assert!(json["avatar_url"].is_null());
    }

    // --- Serialization tests: DeleteAccountResponse ---

    #[test]
    fn delete_account_response_serialization() {
        let resp = DeleteAccountResponse {
            job_id: uuid::Uuid::new_v4().to_string(),
            status: crate::models::personal_erasure::ErasureStatus::Accepted,
            accepted_at: "2025-06-15T14:00:00+00:00".to_string(),
            completed_at: None,
            domains: vec![],
            effects: vec![],
            next_effect_after: None,
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["status"], "accepted");
        assert!(json["completed_at"].is_null());
        assert_eq!(json["accepted_at"], "2025-06-15T14:00:00+00:00");
    }

    // --- Deserialization tests: UpdateProfileRequest ---

    #[test]
    fn update_profile_request_deserialization_all_fields() {
        let json = serde_json::json!({
            "display_name": "New Name",
            "avatar_url": "https://example.com/new-avatar.png"
        });
        let req: UpdateProfileRequest = serde_json::from_value(json).unwrap();

        assert_eq!(req.display_name.as_deref(), Some("New Name"));
        assert_eq!(
            req.avatar_url.as_deref(),
            Some("https://example.com/new-avatar.png")
        );
    }

    #[test]
    fn update_profile_request_deserialization_empty_body() {
        let json = serde_json::json!({});
        let req: UpdateProfileRequest = serde_json::from_value(json).unwrap();

        assert!(req.display_name.is_none());
        assert!(req.avatar_url.is_none());
    }

    #[test]
    fn update_profile_request_deserialization_partial() {
        let json = serde_json::json!({
            "display_name": "Only Name"
        });
        let req: UpdateProfileRequest = serde_json::from_value(json).unwrap();

        assert_eq!(req.display_name.as_deref(), Some("Only Name"));
        assert!(req.avatar_url.is_none());
    }

    // --- Deserialization tests: CompleteOnboardingRequest ---

    #[test]
    fn complete_onboarding_request_deserialization() {
        let json = serde_json::json!({ "key": "ai_services" });
        let req: CompleteOnboardingRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.key, "ai_services");
    }

    #[test]
    fn complete_onboarding_request_deserialization_unknown_key_accepted() {
        // Deserialization accepts any string; validation happens in handler
        let json = serde_json::json!({ "key": "unknown_flow" });
        let req: CompleteOnboardingRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.key, "unknown_flow");
    }

    // --- Structural tests: response field coverage ---

    #[test]
    fn user_profile_response_has_all_expected_json_keys() {
        let resp = UserProfileResponse {
            user_type: crate::models::user::UserType::Person,
            id: "u1".to_string(),
            email: "e@e.com".to_string(),
            display_name: None,
            avatar_url: None,
            email_verified: false,
            mfa_enabled: false,
            is_admin: false,
            is_operator: false,
            role: "user".to_string(),
            is_active: true,
            social_provider: None,
            has_password: false,
            created_at: "2025-01-01T00:00:00+00:00".to_string(),
            updated_at: "2025-01-01T00:00:00+00:00".to_string(),
            last_login_at: None,
            profile_config: ProfileConfigResponse {
                services_view: None,
                service_views: None,
                onboarding: OnboardingStateResponse {
                    ai_services_completed_at: None,
                },
            },
            capabilities: UserCapabilitiesResponse {
                billing_available: false,
                manage_credit_grants: false,
                manage_catalog: false,
                manage_tools: false,
                read_admin_usage: false,
                read_feature_flags: false,
                create_feature_flags: false,
                manage_all_feature_flags: false,
                enabled_features: vec![],
            },
        };
        let json = serde_json::to_value(&resp).unwrap();
        let obj = json.as_object().unwrap();

        let expected_keys = vec![
            "user_type",
            "id",
            "email",
            "display_name",
            "avatar_url",
            "email_verified",
            "mfa_enabled",
            "is_admin",
            "is_operator",
            "role",
            "is_active",
            "social_provider",
            "has_password",
            "created_at",
            "updated_at",
            "last_login_at",
            "profile_config",
            "capabilities",
        ];
        for key in &expected_keys {
            assert!(obj.contains_key(*key), "Missing expected key: {}", key);
        }
        assert_eq!(obj.len(), expected_keys.len());
    }
}

#[cfg(test)]
#[path = "account_deletion_policy_tests.rs"]
mod account_deletion_policy_tests;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalyticsConsentRequest {
    pub account_id: String,
    pub enabled: bool,
}

#[derive(serde::Serialize)]
pub struct AnalyticsConsentResponse {
    pub account_id: String,
    pub policy_version: Option<String>,
    pub revision: String,
    pub enabled: bool,
    pub changed_at: Option<String>,
}

fn analytics_consent_response(
    account_id: &str,
    value: crate::models::user::AnalyticsConsent,
) -> (HeaderMap, Json<AnalyticsConsentResponse>) {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        "no-store".parse().unwrap(),
    );
    (
        headers,
        Json(AnalyticsConsentResponse {
            account_id: account_id.to_owned(),
            policy_version: value.policy_version,
            revision: value.revision,
            enabled: value.enabled,
            changed_at: value.changed_at.map(|time| time.to_rfc3339()),
        }),
    )
}

pub async fn get_analytics_consent(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<(HeaderMap, Json<AnalyticsConsentResponse>)> {
    super::login_client_context::require_first_party_human(&auth_user)?;
    let actor = auth_user.user_id.to_string();
    Ok(analytics_consent_response(
        &actor,
        crate::services::analytics_consent_service::read(&state.db, &actor).await?,
    ))
}

pub async fn set_analytics_consent(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<AnalyticsConsentRequest>,
) -> AppResult<(HeaderMap, Json<AnalyticsConsentResponse>)> {
    super::login_client_context::require_first_party_human(&auth_user)?;
    let actor = auth_user.user_id.to_string();
    if body.account_id != actor {
        return Err(AppError::Forbidden(
            "Account changed; reload before changing analytics permission".into(),
        ));
    }
    Ok(analytics_consent_response(
        &actor,
        crate::services::analytics_consent_service::set(&state.db, &actor, body.enabled).await?,
    ))
}

#[cfg(test)]
mod analytics_tests {
    use super::*;
    use crate::services::analytics_consent_service as consent;
    use crate::{models::user::UserType, test_utils::*};

    #[tokio::test]
    async fn mismatched_subject_cannot_change_either_account_and_agent_cannot_opt_in() {
        let db = connect_test_database("analytics_intent")
            .await
            .expect("MongoDB required");
        let first = uuid::Uuid::new_v4().to_string();
        let second = uuid::Uuid::new_v4().to_string();
        for id in [&first, &second] {
            db.collection::<User>(USERS)
                .insert_one(test_user(id, UserType::Person))
                .await
                .unwrap();
        }
        let a = consent::set(&db, &first, false).await.unwrap();
        let b = consent::set(&db, &second, false).await.unwrap();
        let state = test_app_state(db.clone());
        assert!(
            set_analytics_consent(
                State(state.clone()),
                test_auth_user(&second),
                Json(AnalyticsConsentRequest {
                    account_id: first.clone(),
                    enabled: true
                })
            )
            .await
            .is_err()
        );
        assert_eq!(consent::read(&db, &first).await.unwrap(), a);
        assert_eq!(consent::read(&db, &second).await.unwrap(), b);
        let mut agent = test_auth_user(&first);
        agent.auth_method = crate::mw::auth::AuthMethod::ApiKey;
        agent.api_key_id = Some(uuid::Uuid::new_v4().to_string());
        assert!(
            set_analytics_consent(
                State(state.clone()),
                agent,
                Json(AnalyticsConsentRequest {
                    account_id: first.clone(),
                    enabled: true
                })
            )
            .await
            .is_err()
        );
        assert_eq!(consent::read(&db, &first).await.unwrap(), a);
        let (headers, Json(saved)) = set_analytics_consent(
            State(state),
            test_auth_user(&first),
            Json(AnalyticsConsentRequest {
                account_id: first.clone(),
                enabled: true,
            }),
        )
        .await
        .unwrap();
        assert!(saved.enabled);
        assert_eq!(saved.account_id, first);
        assert_eq!(
            saved.policy_version.as_deref(),
            Some(consent::POLICY_VERSION)
        );
        assert!(!saved.revision.is_empty());
        assert_eq!(headers.get("cache-control").unwrap(), "no-store");
        assert_eq!(consent::read(&db, &second).await.unwrap(), b);
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientAnalyticsRequest {
    pub account_id: String,
    pub revision: String,
    pub event: crate::telemetry::client_event::ClientEvent,
}

/// Limits the raw relay body before Json buffers/deserializes it. The event
/// itself has a stricter 4KiB limit after decoding; this includes its envelope.
pub fn analytics_body_limit() -> axum::extract::DefaultBodyLimit {
    axum::extract::DefaultBodyLimit::max(8 * 1024)
}

/// The authenticated subject and captured revision fence cross-tab cookie
/// switches and queued work. Device headers never grant account permission.
pub async fn capture_analytics(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Json(body): Json<ClientAnalyticsRequest>,
) -> AppResult<axum::http::StatusCode> {
    super::login_client_context::require_first_party_human(&auth_user)?;
    let actor = auth_user.user_id.to_string();
    if body.account_id != actor {
        return Err(AppError::Forbidden("Analytics account changed".into()));
    }
    if body.revision.len() > 64 || body.revision.is_empty() {
        return Err(AppError::ValidationError(
            "Analytics revision missing or invalid".into(),
        ));
    }
    body.event.properties()?;
    emit_event(
        state.telemetry.as_deref(),
        &actor,
        None,
        &tele,
        TelemetryEvent::ClientOriginated {
            event: body.event,
            revision: body.revision,
        },
    )
    .await;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod analytics_ingress_tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        routing::post,
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn relay_json_ingress_bounds_body_and_rejects_vendor_controls() {
        // The same DTO, validator and pre-deserialization limit used by the
        // authenticated route, isolated here from authentication for ingress.
        let app = Router::new().route(
            "/",
            post(|Json(body): Json<ClientAnalyticsRequest>| async move {
                body.event.properties().map(|_| StatusCode::NO_CONTENT)
            })
            .layer(analytics_body_limit()),
        );
        let send = |body: String| {
            app.clone().oneshot(
                Request::post("/")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
        };
        let envelope = |event: serde_json::Value| {
            serde_json::json!({
            "account_id": uuid::Uuid::new_v4().to_string(), "revision": "revision", "event": event
        }).to_string()
        };
        assert_eq!(
            send(envelope(
                serde_json::json!({"name":"ui.docs_opened", "props":{"page":"encryption"}})
            ))
            .await
            .unwrap()
            .status(),
            StatusCode::NO_CONTENT
        );
        for event in [
            serde_json::json!({"name":"$identify", "props":{}}),
            serde_json::json!({"name":"ui.docs_opened", "props":{}, "distinct_id":"other"}),
        ] {
            assert_eq!(
                send(envelope(event)).await.unwrap().status(),
                StatusCode::UNPROCESSABLE_ENTITY
            );
        }
        for key in ["$set", "$set_once", "$groups", "$process_person_profile"] {
            let mut props = serde_json::Map::new();
            props.insert(
                key.into(),
                serde_json::json!({"secret":"distinctive-value"}),
            );
            assert_eq!(
                send(envelope(
                    serde_json::json!({"name":"ui.docs_opened", "props": props})
                ))
                .await
                .unwrap()
                .status(),
                StatusCode::BAD_REQUEST
            );
        }
        let oversized = envelope(
            serde_json::json!({"name":"ui.docs_opened", "props":{"page":"x".repeat(9000)}}),
        );
        assert_eq!(
            send(oversized).await.unwrap().status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        // Chunked streams with no Content-Length are bounded too.
        let stream = futures::stream::iter(
            (0..10).map(|_| Ok::<_, std::io::Error>(bytes::Bytes::from(vec![b' '; 1024]))),
        );
        let response = app
            .oneshot(
                Request::post("/")
                    .header("content-type", "application/json")
                    .body(Body::from_stream(stream))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}
