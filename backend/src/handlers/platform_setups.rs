use crate::services::{audit_service, platform_setup_service as setups};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{credits::Credits, platform_setup::*},
    mw::auth::AuthUser,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, header},
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSetup {
    pub request_id: uuid::Uuid,
    pub template: SetupTemplate,
    pub facts: SetupFacts,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsInput {
    pub daily_credits: Credits,
    pub turns_per_hour: u32,
    pub concurrent_turns: u32,
    pub max_input_chars: u32,
    pub max_output_tokens: u32,
}
impl From<RuntimeLimits> for LimitsInput {
    fn from(value: RuntimeLimits) -> Self {
        Self {
            daily_credits: value.daily_credits,
            turns_per_hour: value.turns_per_hour,
            concurrent_turns: value.concurrent_turns,
            max_input_chars: value.max_input_chars,
            max_output_tokens: value.max_output_tokens,
        }
    }
}
impl From<LimitsInput> for RuntimeLimits {
    fn from(value: LimitsInput) -> Self {
        Self {
            daily_credits: value.daily_credits,
            turns_per_hour: value.turns_per_hour,
            concurrent_turns: value.concurrent_turns,
            max_input_chars: value.max_input_chars,
            max_output_tokens: value.max_output_tokens,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditSetup {
    pub revision: i64,
    pub facts: SetupFacts,
    pub limits: LimitsInput,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionInput {
    pub revision: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachBindingInput {
    pub revision: i64,
    pub binding_id: uuid::Uuid,
    pub binding_generation: i64,
}
pub async fn attach_binding(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<AttachBindingInput>,
) -> AppResult<(HeaderMap, Json<SetupResponse>)> {
    let row = setups::attach_binding(
        &state.db,
        &actor(&auth)?,
        &id.to_string(),
        body.revision,
        &body.binding_id.to_string(),
        body.binding_generation,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "platform_setup_binding_adopted",
        Some(serde_json::json!({"setup_id": row.id, "revision": row.revision})),
    );
    Ok((headers(), Json(response(&state, row).await?)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveQuery {
    pub before_ms: Option<i64>,
    pub before_id: Option<uuid::Uuid>,
}
#[derive(Serialize)]
pub struct ArchiveResponse {
    pub setups: Vec<SetupResponse>,
    pub next_before_ms: Option<i64>,
    pub next_before_id: Option<String>,
}

pub async fn archived(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ArchiveQuery>,
) -> AppResult<(HeaderMap, Json<ArchiveResponse>)> {
    let before = match (query.before_ms, query.before_id) {
        (None, None) => None,
        (Some(at), Some(id)) => Some((
            chrono::DateTime::from_timestamp_millis(at)
                .ok_or_else(|| AppError::ValidationError("Invalid archive cursor".into()))?,
            id.to_string(),
        )),
        _ => {
            return Err(AppError::ValidationError(
                "Both archive cursor fields are required".into(),
            ));
        }
    };
    let mut rows = setups::list_archived(&state.db, &actor(&auth)?, before).await?;
    let more = rows.len() > 20;
    rows.truncate(20);
    let cursor = if more {
        rows.last()
            .map(|r| (r.updated_at.timestamp_millis(), r.id.clone()))
    } else {
        None
    };
    let mut output = Vec::with_capacity(rows.len());
    for row in rows {
        output.push(response(&state, row).await?);
    }
    Ok((
        headers(),
        Json(ArchiveResponse {
            setups: output,
            next_before_ms: cursor.as_ref().map(|c| c.0),
            next_before_id: cursor.map(|c| c.1),
        }),
    ))
}

#[derive(Serialize)]
pub struct SetupResponse {
    pub id: String,
    pub template: SetupTemplate,
    pub template_version: i64,
    pub revision: i64,
    pub state: SetupState,
    pub facts: SetupFacts,
    pub limits: LimitsInput,
    pub agent_id: Option<String>,
    pub channel_bot_id: Option<String>,
    pub creation_request_id: Option<String>,
    pub observed_telegram_bot_id: Option<String>,
    pub publication_revision: Option<i64>,
    pub approved: bool,
    pub delivery_tested: bool,
    pub expires_at: Option<String>,
    pub updated_at: String,
    pub runtime_notice: Option<String>,
    pub effective_state: SetupState,
}

fn headers() -> HeaderMap {
    HeaderMap::from_iter([
        (header::CACHE_CONTROL, "no-store".parse().unwrap()),
        (header::REFERRER_POLICY, "no-referrer".parse().unwrap()),
    ])
}
fn actor(auth: &AuthUser) -> AppResult<String> {
    super::login_client_context::require_first_party_human(auth)?;
    Ok(auth.user_id.to_string())
}
async fn response(state: &AppState, row: PlatformSetup) -> AppResult<SetupResponse> {
    let facts = setups::decrypt_facts(&state.encryption_keys, &row).await?;
    let runtime_notice = setups::runtime_notice(&state.db, &row)
        .await?
        .map(str::to_owned);
    Ok(SetupResponse {
        effective_state: if runtime_notice.is_some() {
            SetupState::Paused
        } else {
            row.state
        },
        runtime_notice,
        approved: row.approved_revision == Some(row.revision) && row.approved_digest.is_some(),
        id: row.id,
        template: row.template,
        template_version: row.template_version,
        revision: row.revision,
        state: row.state,
        facts,
        limits: row.limits.into(),
        agent_id: row.resources.agent_id,
        channel_bot_id: row.resources.channel_bot_id,
        creation_request_id: row.resources.creation_request_id,
        observed_telegram_bot_id: row.resources.observed_telegram_bot_id,
        publication_revision: row.published.map(|p| p.revision),
        delivery_tested: row.delivery_tested,
        expires_at: row.expires_at.map(|t| t.to_rfc3339()),
        updated_at: row.updated_at.to_rfc3339(),
    })
}

pub async fn templates() -> Json<[setups::TemplateDescription; 2]> {
    Json(setups::templates())
}
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateSetup>,
) -> AppResult<(HeaderMap, Json<SetupResponse>)> {
    let actor = actor(&auth)?;
    if !crate::mw::rate_limit::PerKeyRateLimiter::with_db(
        state.db.clone(),
        "platform_setup_create",
        10,
        60,
    )
    .check_shared(&actor)
    .await?
    {
        return Err(AppError::RateLimited);
    }
    let row = setups::create_owned(
        &state.db,
        &state.encryption_keys,
        &actor,
        &body.request_id.to_string(),
        body.template,
        &body.facts,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "platform_setup_created",
        Some(serde_json::json!({"setup_id": row.id, "template": row.template})),
    );
    Ok((headers(), Json(response(&state, row).await?)))
}
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<(HeaderMap, Json<Vec<SetupResponse>>)> {
    let rows = setups::list_owned(&state.db, &actor(&auth)?).await?;
    let mut output = Vec::with_capacity(rows.len());
    for row in rows {
        output.push(response(&state, row).await?);
    }
    Ok((headers(), Json(output)))
}
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
) -> AppResult<(HeaderMap, Json<SetupResponse>)> {
    Ok((
        headers(),
        Json(
            response(
                &state,
                setups::get_owned(&state.db, &actor(&auth)?, &id.to_string()).await?,
            )
            .await?,
        ),
    ))
}
pub async fn edit(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<EditSetup>,
) -> AppResult<(HeaderMap, Json<SetupResponse>)> {
    let row = setups::edit_owned(
        &state.db,
        &state.encryption_keys,
        &actor(&auth)?,
        &id.to_string(),
        body.revision,
        &body.facts,
        &body.limits.into(),
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "platform_setup_edited",
        Some(serde_json::json!({"setup_id": row.id, "revision": row.revision})),
    );
    Ok((headers(), Json(response(&state, row).await?)))
}
pub async fn review(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<RevisionInput>,
) -> AppResult<(HeaderMap, Json<SetupResponse>)> {
    let row = setups::request_review(
        &state.db,
        &state.encryption_keys,
        &actor(&auth)?,
        &id.to_string(),
        body.revision,
    )
    .await?;
    Ok((headers(), Json(response(&state, row).await?)))
}
pub async fn publish(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<RevisionInput>,
) -> AppResult<(HeaderMap, Json<SetupResponse>)> {
    let row = setups::publish_reviewed(
        &state.db,
        &state.encryption_keys,
        &actor(&auth)?,
        &id.to_string(),
        body.revision,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "platform_setup_published",
        Some(serde_json::json!({"setup_id": row.id, "revision": row.revision})),
    );
    Ok((headers(), Json(response(&state, row).await?)))
}
pub async fn pause(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<RevisionInput>,
) -> AppResult<(HeaderMap, Json<SetupResponse>)> {
    let row = setups::pause_or_cancel(
        &state.db,
        &actor(&auth)?,
        &id.to_string(),
        body.revision,
        false,
    )
    .await?;
    Ok((headers(), Json(response(&state, row).await?)))
}
pub async fn cancel(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<RevisionInput>,
) -> AppResult<(HeaderMap, Json<SetupResponse>)> {
    let row = setups::pause_or_cancel(
        &state.db,
        &actor(&auth)?,
        &id.to_string(),
        body.revision,
        true,
    )
    .await?;
    Ok((headers(), Json(response(&state, row).await?)))
}

#[derive(Serialize)]
pub struct ApprovalResponse {
    pub setup: SetupResponse,
    pub digest: String,
    pub reviewed_revision: i64,
    pub request_revision: Option<i64>,
    pub child_bot_id: Option<i64>,
    pub child_username: Option<String>,
    pub manager_revision: Option<i64>,
    pub agent_id: String,
    pub creation_request_id: String,
    pub model_service_id: String,
    pub model_service_name: Option<String>,
    pub model_id: String,
    pub specialist_name: String,
    pub funding_owner_user_id: String,
    pub funding_owner_name: Option<String>,
    pub connector_grants: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalInput {
    pub revision: i64,
    pub digest: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildApprovalInput {
    pub revision: i64,
    pub request_revision: i64,
    pub child_bot_id: i64,
    pub manager_revision: i64,
    pub digest: String,
}

pub async fn approval(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
) -> AppResult<(HeaderMap, Json<ApprovalResponse>)> {
    let owner = actor(&auth)?;
    let row = setups::get_owned(&state.db, &owner, &id.to_string()).await?;
    let (review, request, digest) = if row.state == SetupState::AwaitingChildReview {
        let (candidate, request, digest) =
            crate::services::platform_provisioning_service::child_candidate(
                &state.db,
                &state.encryption_keys,
                &owner,
                &row.id,
            )
            .await?;
        (candidate, Some(request), digest)
    } else if row.state == SetupState::AwaitingApproval {
        let facts = setups::decrypt_facts(&state.encryption_keys, &row).await?;
        let digest = setups::approval_digest(&row, &facts)?;
        (row.clone(), None, digest)
    } else {
        return Err(AppError::Conflict(
            "This setup is not awaiting review".into(),
        ));
    };
    let missing = || AppError::Conflict("Review the current setup resource plan".into());
    let model_service_name = match review.resources.runtime_model_service_id.as_deref() {
        Some(service) => state
            .db
            .collection::<mongodb::bson::Document>(
                crate::models::downstream_service::COLLECTION_NAME,
            )
            .find_one(mongodb::bson::doc! {"_id": service})
            .await?
            .and_then(|row| row.get_str("name").ok().map(str::to_owned)),
        None => None,
    };
    let funding_owner_name = state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .find_one(mongodb::bson::doc! {"_id": &owner, "is_active": true})
        .await?
        .and_then(|user| user.display_name);
    let specialist_name = setups::decrypt_facts(&state.encryption_keys, &review)
        .await?
        .name;
    let output = ApprovalResponse {
        reviewed_revision: row.revision,
        digest,
        model_service_name,
        funding_owner_name,
        specialist_name,
        request_revision: request.as_ref().map(|r| r.revision),
        child_bot_id: request.as_ref().and_then(|r| r.telegram_bot_id),
        child_username: request.as_ref().and_then(|r| r.bot_username.clone()),
        manager_revision: request.as_ref().and_then(|r| r.manager_revision),
        agent_id: review.resources.agent_id.clone().ok_or_else(missing)?,
        creation_request_id: review
            .resources
            .creation_request_id
            .clone()
            .ok_or_else(missing)?,
        model_service_id: review
            .resources
            .runtime_model_service_id
            .clone()
            .ok_or_else(missing)?,
        model_id: review
            .resources
            .runtime_model_id
            .clone()
            .ok_or_else(missing)?,
        funding_owner_user_id: owner,
        connector_grants: vec![],
        setup: response(&state, row).await?,
    };
    Ok((headers(), Json(output)))
}

pub async fn approve(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<ApprovalInput>,
) -> AppResult<(HeaderMap, Json<SetupResponse>)> {
    let row = crate::services::platform_provisioning_service::approve_owned(
        &state.db,
        &state.encryption_keys,
        &actor(&auth)?,
        &id.to_string(),
        body.revision,
        &body.digest,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "platform_setup_approved",
        Some(serde_json::json!({"setup_id": row.id, "revision": row.revision})),
    );
    Ok((headers(), Json(response(&state, row).await?)))
}
pub async fn approve_child(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<ChildApprovalInput>,
) -> AppResult<(HeaderMap, Json<SetupResponse>)> {
    let row = crate::services::platform_provisioning_service::approve_child_owned(
        &state.db,
        &state.encryption_keys,
        &actor(&auth)?,
        &id.to_string(),
        body.revision,
        body.request_revision,
        body.child_bot_id,
        body.manager_revision,
        &body.digest,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "platform_setup_child_approved",
        Some(serde_json::json!({"setup_id": row.id, "revision": row.revision})),
    );
    Ok((headers(), Json(response(&state, row).await?)))
}

#[derive(Serialize)]
pub struct LaunchResponse {
    pub url: String,
}
pub async fn launch_creation(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
) -> AppResult<(HeaderMap, Json<LaunchResponse>)> {
    let owner = actor(&auth)?;
    let setup = setups::get_owned(&state.db, &owner, &id.to_string()).await?;
    let request = setup
        .resources
        .creation_request_id
        .ok_or_else(|| AppError::Conflict("Creation is not ready yet".into()))?;
    let service = crate::services::telegram_new_service::TelegramNewService {
        manager_id: None,
        db: &state.db,
        keys: &state.encryption_keys,
        config: &state.config,
        api: crate::services::telegram_new_api::TelegramApi::new(&state.http_client),
    };
    let url = service.launch(&owner, &request).await?;
    Ok((headers(), Json(LaunchResponse { url })))
}

pub async fn activate(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<RevisionInput>,
) -> AppResult<(HeaderMap, Json<SetupResponse>)> {
    let row = crate::services::platform_runtime_service::activate_owned(
        &state,
        &actor(&auth)?,
        &id.to_string(),
        body.revision,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "platform_setup_activated",
        Some(serde_json::json!({"setup_id": row.id, "revision": row.revision})),
    );
    Ok((headers(), Json(response(&state, row).await?)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnonymousDraftQuery {
    pub binding_id: uuid::Uuid,
    pub binding_generation: i64,
}
pub async fn anonymous_drafts(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(body): Query<AnonymousDraftQuery>,
) -> AppResult<(HeaderMap, Json<Vec<SetupResponse>>)> {
    let rows = setups::anonymous_for_owner(
        &state,
        &actor(&auth)?,
        &body.binding_id.to_string(),
        body.binding_generation,
    )
    .await?;
    let mut output = Vec::with_capacity(rows.len());
    for row in rows {
        output.push(response(&state, row).await?);
    }
    Ok((headers(), Json(output)))
}
pub async fn adopt_draft(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<AttachBindingInput>,
) -> AppResult<(HeaderMap, Json<SetupResponse>)> {
    let row = setups::adopt_anonymous(
        &state,
        &actor(&auth)?,
        &id.to_string(),
        body.revision,
        &body.binding_id.to_string(),
        body.binding_generation,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "platform_anonymous_draft_adopted",
        Some(serde_json::json!({"setup_id": row.id, "revision": row.revision})),
    );
    Ok((headers(), Json(response(&state, row).await?)))
}
