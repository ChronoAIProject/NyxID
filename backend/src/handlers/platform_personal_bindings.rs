//! Human-only personal consent endpoints. Part 5 mounts these after auth guards.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::platform_link_intent::{ChallengeState, PlatformLinkChallenge},
    mw::auth::AuthUser,
    services::{
        platform_link_intent_service as proofs, platform_personal_binding_service as bindings,
    },
};
use axum::{
    Extension, Json,
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, header},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentInput {
    pub intent_token: Zeroizing<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChallengeInput {
    pub challenge_id: uuid::Uuid,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApproveInput {
    pub challenge_id: uuid::Uuid,
    pub review_digest: String,
    #[serde(default)]
    pub replace_existing: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeInput {
    pub generation: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListInput {
    pub before_id: Option<uuid::Uuid>,
}
#[derive(Serialize)]
pub struct ManagerResponse {
    pub id: String,
    pub username: Option<String>,
    pub display_name: String,
}
#[derive(Serialize)]
pub struct SenderResponse {
    pub id: String,
    pub display_name: Option<String>,
}
#[derive(Serialize)]
pub struct PreviewResponse {
    pub purpose: &'static str,
    pub status: &'static str,
    pub manager: ManagerResponse,
    pub sender: SenderResponse,
    pub expires_at: String,
    pub invitation_required: bool,
    pub requires_same_chat_confirmation: bool,
}
#[derive(Serialize)]
pub struct AccountResponse {
    pub id: String,
    pub display_name: Option<String>,
}
#[derive(Serialize)]
pub struct Disclosures {
    pub telegram_transport: bool,
    pub existing_nyxbot_access: bool,
    pub can_approve_sensitive_actions: bool,
    pub started_work_may_continue_after_unlink: bool,
}
#[derive(Serialize)]
pub struct ReviewResponse {
    pub challenge_id: String,
    pub status: &'static str,
    pub expires_at: String,
    pub review_digest: String,
    pub account: AccountResponse,
    pub permission_mode: String,
    pub billing: &'static str,
    pub replaces_existing_binding: bool,
    pub disclosures: Disclosures,
}
#[derive(Serialize)]
pub struct BindingResponse {
    pub id: String,
    pub generation: i64,
}
#[derive(Serialize)]
pub struct StatusResponse {
    pub challenge_id: String,
    pub status: &'static str,
    pub expires_at: String,
    pub binding: Option<BindingResponse>,
    pub confirmation_command: Option<Zeroizing<String>>,
}

fn private_headers() -> HeaderMap {
    HeaderMap::from_iter([
        (header::CACHE_CONTROL, "no-store".parse().unwrap()),
        (header::REFERRER_POLICY, "no-referrer".parse().unwrap()),
    ])
}
fn manager(id: String) -> ManagerResponse {
    ManagerResponse {
        id,
        username: None,
        display_name: "NyxBot".into(),
    }
}
fn project_status(row: PlatformLinkChallenge) -> StatusResponse {
    StatusResponse {
        challenge_id: row.id,
        status: match row.state {
            ChallengeState::Review => "review",
            ChallengeState::AwaitingChatConfirmation => "awaiting_chat_confirmation",
            ChallengeState::Confirmed => "confirmed",
            ChallengeState::Denied => "denied",
            ChallengeState::Cancelled => "cancelled",
            ChallengeState::Expired => "expired",
        },
        expires_at: row.expires_at.to_rfc3339(),
        binding: row.completion.map(|b| BindingResponse {
            id: b.binding_id,
            generation: b.binding_generation,
        }),
        confirmation_command: None,
    }
}

pub async fn preview(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<IntentInput>,
) -> AppResult<(HeaderMap, Json<PreviewResponse>)> {
    let ip = super::login_client_context::resolve_client_ip(&headers, peer, &state)?;
    if !crate::services::coordination_service::RateWindowStore::admit(
        &state.db,
        "personal_intent_preview_ip",
        &ip.to_string(),
        30,
        std::time::Duration::from_secs(60),
    )
    .await?
    .allowed
    {
        return Err(AppError::RateLimited);
    }
    let row = proofs::preview(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        &input.intent_token,
    )
    .await?;
    Ok((
        private_headers(),
        Json(PreviewResponse {
            purpose: "personal_nyxbot",
            status: "pending",
            manager: manager(row.platform_config_id),
            sender: SenderResponse {
                id: row.telegram_sender_id.to_string(),
                display_name: None,
            },
            expires_at: row.expires_at.to_rfc3339(),
            invitation_required: crate::services::feature_flag_service::invitation_code_required(
                &state.db,
            )
            .await?,
            requires_same_chat_confirmation: true,
        }),
    ))
}

pub async fn review(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<IntentInput>,
) -> AppResult<(HeaderMap, Json<ReviewResponse>)> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    let row = proofs::review(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        &actor,
        &input.intent_token,
    )
    .await?;
    let user = state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .find_one(bson::doc! {"_id":&actor,"is_active":true,"user_type":"person"})
        .await?
        .ok_or_else(bindings::unavailable)?;
    let replaces = row.previous_binding.as_ref().is_some_and(|b| {
        b.state == crate::models::platform_personal_binding::PersonalBindingState::Active
            && b.user_id != actor
    });
    Ok((
        private_headers(),
        Json(ReviewResponse {
            challenge_id: row.id,
            status: "review",
            expires_at: row.expires_at.to_rfc3339(),
            review_digest: row.review_digest,
            account: AccountResponse {
                id: actor,
                display_name: user.display_name,
            },
            permission_mode: row.reviewed_access_mode,
            billing: "personal",
            replaces_existing_binding: replaces,
            disclosures: Disclosures {
                telegram_transport: true,
                existing_nyxbot_access: true,
                can_approve_sensitive_actions: true,
                started_work_may_continue_after_unlink: true,
            },
        }),
    ))
}

pub async fn approve(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<ApproveInput>,
) -> AppResult<(HeaderMap, Json<StatusResponse>)> {
    super::login_client_context::require_first_party_human(&auth)?;
    let result = proofs::approve(
        &state.db,
        state.auth_device_hmac_key.as_slice(),
        &auth.user_id.to_string(),
        &input.challenge_id.to_string(),
        &input.review_digest,
        input.replace_existing,
    )
    .await?;
    let mut response = project_status(result.challenge);
    response.confirmation_command = result.confirmation_command;
    Ok((private_headers(), Json(response)))
}

pub async fn status(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<ChallengeInput>,
) -> AppResult<(HeaderMap, Json<StatusResponse>)> {
    super::login_client_context::require_first_party_human(&auth)?;
    Ok((
        private_headers(),
        Json(project_status(
            proofs::status(
                &state.db,
                &auth.user_id.to_string(),
                &input.challenge_id.to_string(),
            )
            .await?,
        )),
    ))
}

#[derive(Serialize)]
pub struct DeniedResponse {
    pub status: &'static str,
}
pub async fn deny(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<ChallengeInput>,
) -> AppResult<(HeaderMap, Json<DeniedResponse>)> {
    super::login_client_context::require_first_party_human(&auth)?;
    proofs::deny(
        &state.db,
        &auth.user_id.to_string(),
        &input.challenge_id.to_string(),
    )
    .await?;
    Ok((private_headers(), Json(DeniedResponse { status: "denied" })))
}

#[derive(Serialize)]
pub struct OwnedBindingResponse {
    pub id: String,
    pub generation: i64,
    pub manager: ManagerResponse,
    pub sender: SenderResponse,
    pub state: crate::models::platform_personal_binding::PersonalBindingState,
    pub conversation_id: Option<String>,
    pub consented_at: String,
    pub revoked_at: Option<String>,
}
#[derive(Serialize)]
pub struct ListResponse {
    pub bindings: Vec<OwnedBindingResponse>,
    pub next_before_id: Option<String>,
}
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(input): Query<ListInput>,
) -> AppResult<(HeaderMap, Json<ListResponse>)> {
    super::login_client_context::require_first_party_human(&auth)?;
    let cursor = input.before_id.map(|id| id.to_string());
    let mut rows =
        bindings::list_owned(&state.db, &auth.user_id.to_string(), cursor.as_deref()).await?;
    let more = rows.len() > 50;
    rows.truncate(50);
    let next_before_id = if more {
        rows.last().map(|r| r.id.clone())
    } else {
        None
    };
    let rows = rows
        .into_iter()
        .map(|row| OwnedBindingResponse {
            id: row.id,
            generation: row.generation,
            manager: manager(row.platform_config_id),
            sender: SenderResponse {
                id: row.telegram_sender_id.to_string(),
                display_name: None,
            },
            state: row.state,
            conversation_id: row.conversation_id,
            consented_at: row.consented_at.to_rfc3339(),
            revoked_at: row.revoked_at.map(|d| d.to_rfc3339()),
        })
        .collect();
    Ok((
        private_headers(),
        Json(ListResponse {
            bindings: rows,
            next_before_id,
        }),
    ))
}

#[derive(Serialize)]
pub struct RevokedResponse {
    pub revoked: bool,
    pub work_may_continue_in_app: bool,
}
pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Extension(producers): Extension<
        Arc<crate::services::platform_manager_contract::PlatformManagerProducers>,
    >,
    Path(id): Path<uuid::Uuid>,
    Json(input): Json<RevokeInput>,
) -> AppResult<(HeaderMap, Json<RevokedResponse>)> {
    super::login_client_context::require_first_party_human(&auth)?;
    if input.generation <= 0 {
        return Err(AppError::ValidationError(
            "Invalid binding generation".into(),
        ));
    }
    bindings::revoke_owned(
        &state.db,
        &crate::services::audit_service::AuditActor::from_auth_user(&auth),
        state.audit_chain_hmac_key.as_slice(),
        &id.to_string(),
        input.generation,
        &bindings::LifecycleProducer {
            personal: producers.personal.clone(),
            public: producers.public.clone(),
        },
    )
    .await?;
    Ok((
        private_headers(),
        Json(RevokedResponse {
            revoked: true,
            work_may_continue_in_app: true,
        }),
    ))
}
