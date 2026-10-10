use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
    services::{
        assistant_acknowledgement_service as acks, assistant_agent_learning as learning,
        assistant_agent_learning_review as review, assistant_nyxagent as engine,
        assistant_team_service as team,
    },
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionBody {
    #[serde(default)]
    acknowledgement_id: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    revision: Option<i64>,
    #[serde(default)]
    agent_skills_revision: Option<i64>,
    #[serde(default)]
    renewal_of: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditBody {
    draft: Value,
}

/// Other query parameters are ignored, as before the card parameter existed.
#[derive(Deserialize)]
pub struct PreviewQuery {
    #[serde(default)]
    acknowledgement_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigureBody {
    enabled: bool,
    threshold: i64,
}

pub async fn status(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(agent_id): Path<String>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    engine::require_enabled(&state.db, &actor).await?;
    let agent = team::agent(&state.db, &actor, &agent_id).await?;
    crate::services::org_agent_service::require_use(&state.db, &actor, &agent).await?;
    let config = state
        .db
        .collection::<crate::models::assistant_agent_learning::AssistantAgentLearning>(
            crate::models::assistant_agent_learning::CONFIG_COLLECTION_NAME,
        )
        .find_one(mongodb::bson::doc! {"_id": &agent_id})
        .await?;
    Ok(Json(
        json!({"enabled": config.as_ref().is_some_and(|c| c.enabled), "config": config.map(|c| json!({"threshold": c.threshold, "learning_epoch": c.learning_epoch, "config_revision": c.config_revision, "eligible_count": c.eligible_count, "last_run_at": c.last_run_at, "last_success_at": c.last_success_at, "last_error_code": c.last_error_code}))}),
    ))
}

pub async fn configure(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(agent_id): Path<String>,
    Json(body): Json<ConfigureBody>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    engine::require_enabled(&state.db, &actor).await?;
    let row =
        learning::configure(&state.db, &actor, &agent_id, body.enabled, body.threshold).await?;
    Ok(Json(
        json!({"enabled": row.enabled, "threshold": row.threshold, "learning_epoch": row.learning_epoch, "config_revision": row.config_revision}),
    ))
}

pub async fn consent(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(agent_id): Path<String>,
    Json(body): Json<ConfigureBody>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    engine::require_enabled(&state.db, &actor).await?;
    let row = learning::set_member_opt_in(&state.db, &actor, &agent_id, body.enabled).await?;
    Ok(Json(
        json!({"opted_in": row.opted_in, "revision": row.revision, "learning_epoch": row.learning_epoch}),
    ))
}

async fn chat(state: &AppState, owner: &str) -> AppResult<acks::ChatAuthority> {
    let bot = team::ensure_nyxbot(&state.db, owner).await?;
    let home = team::home_thread_for(&state.db, &state.encryption_keys, owner, &bot).await?;
    acks::for_key(&state.db, owner, Some(&home.credential_api_key_id))
        .await?
        .ok_or_else(|| AppError::Forbidden("NyxBot owner card is unavailable".into()))
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(agent_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    Ok(Json(
        json!({"proposals": review::list(&state, &owner, &agent_id, true).await?}),
    ))
}

pub async fn reject(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((agent_id, proposal_id)): Path<(String, String)>,
    Json(body): Json<DecisionBody>,
) -> AppResult<Json<serde_json::Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    review::reject(
        &state,
        &owner,
        &agent_id,
        &proposal_id,
        body.revision.unwrap_or(0),
        body.reason.as_deref().unwrap_or("rejected"),
    )
    .await?;
    Ok(Json(json!({"status":"rejected"})))
}

pub async fn edit(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((agent_id, proposal_id)): Path<(String, String)>,
    Json(body): Json<EditBody>,
) -> AppResult<Json<serde_json::Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let revision = body
        .draft
        .get("revision")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    review::edit(
        &state,
        &owner,
        &agent_id,
        &proposal_id,
        revision,
        &body.draft,
    )
    .await?;
    Ok(Json(json!({"status": "updated"})))
}

pub async fn approve(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((agent_id, proposal_id)): Path<(String, String)>,
    Json(body): Json<DecisionBody>,
) -> AppResult<Json<serde_json::Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    if body.renewal_of.is_some() && body.acknowledgement_id.is_some() {
        return Err(AppError::Conflict(
            "renewal_of cannot be combined with acknowledgement_id".into(),
        ));
    }
    let proposal_row = state
        .db
        .collection::<crate::models::assistant_agent_learning::AssistantAgentLearningProposal>(
            crate::models::assistant_agent_learning::PROPOSALS_COLLECTION_NAME,
        )
        .find_one(mongodb::bson::doc! {"_id": &proposal_id, "agent_id": &agent_id})
        .await?
        .ok_or_else(|| AppError::NotFound("Learning proposal not found".into()))?;
    // Authored proposals are confirmed only on their conversation card; this
    // route may only renew that card, so it never raises a card elsewhere.
    // Other callers fall through to the binding's ownership refusal.
    if proposal_row.owner_id == owner
        && proposal_row.source == crate::models::assistant_agent_learning::ProposalSource::Authored
        && body.renewal_of.is_none()
    {
        return Err(AppError::Conflict(
            "Confirm this authored skill from its conversation card".into(),
        ));
    }
    let renewal = match body.renewal_of.as_deref() {
        Some(original_id) => Some(
            crate::services::assistant_skill_authoring::renewal_source(
                &state.db,
                &owner,
                &agent_id,
                &proposal_id,
                original_id,
                body.revision,
            )
            .await?,
        ),
        None => None,
    };
    let chat = match &renewal {
        Some((_, original_chat)) => original_chat.clone(),
        None => chat(&state, &owner).await?,
    };
    let expected_revision = body.revision.unwrap_or(proposal_row.revision);
    let expected_skills_revision = if body.renewal_of.is_some() {
        body.agent_skills_revision
            .ok_or_else(|| AppError::Conflict("Skill review changed".into()))?
    } else {
        body.agent_skills_revision
            .unwrap_or(proposal_row.agent_skills_revision)
    };
    let reader = super::agent_skills::Reader {
        state: &state,
        person: &owner,
        thread_key: Some(&chat.api_key_id),
        scopes: None,
        chat: Some(std::sync::Arc::new(chat.clone())),
    };
    let args = review::approval_binding(
        &state,
        &owner,
        &agent_id,
        &proposal_id,
        expected_revision,
        expected_skills_revision,
        &reader,
    )
    .await?;
    if let Some((original, _)) = &renewal {
        review::require_renewal_of(original, &args)?;
    }
    if args["agent_id"].as_str() != Some(&agent_id) {
        return Err(AppError::NotFound("Learning proposal not found".into()));
    }
    if let Some(id) = body.acknowledgement_id.as_deref() {
        review::confirm_learned_card(
            &state,
            &chat,
            &crate::services::audit_service::AuditActor::from_auth_user(&auth),
            id,
            &args,
        )
        .await?;
    } else {
        let card = acks::request(&state.db, &chat, acks::Request { kind: "action", service: None, tool: Some("nyxid__approve_agent_learning"), arguments: Some(&args), summary: "Publish this reviewed private Ornn skill and attach its exact pinned version to the agent.", platform: false }).await?;
        return Ok(Json(
            json!({"status":"confirmation_required", "acknowledgement":acks::refusal(&card)}),
        ));
    }
    Ok(Json(
        review::approve(
            &state,
            &chat,
            &agent_id,
            &proposal_id,
            body.acknowledgement_id.as_deref().unwrap_or_default(),
            &args,
            &reader,
        )
        .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReprepareBody {
    /// The authored card the owner clicked; absent for learned drafts, which
    /// are rebuilt from the review panel.
    #[serde(default)]
    acknowledgement_id: Option<String>,
}

/// Rebuilds a legacy update package with the source skill's interface. The
/// base is read with the owner's own identity: this is a human action.
pub async fn reprepare(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((agent_id, proposal_id)): Path<(String, String)>,
    Json(body): Json<ReprepareBody>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    engine::require_enabled(&state.db, &actor).await?;
    let reader = super::agent_skills::Reader {
        state: &state,
        person: &actor,
        thread_key: None,
        scopes: None,
        chat: None,
    };
    Ok(Json(
        review::reprepare(
            &state,
            &crate::services::audit_service::AuditActor::from_auth_user(&auth),
            &agent_id,
            &proposal_id,
            body.acknowledgement_id.as_deref(),
            &reader,
        )
        .await?,
    ))
}

pub async fn run_now(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(agent_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    Ok(Json(
        json!({"run_id": learning::run_now(&state, &owner, &agent_id).await?}),
    ))
}

/// Content never enters acknowledgement rows, audit or model-visible output.
pub async fn authored_preview(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((agent, id)): Path<(String, String)>,
    Query(query): Query<PreviewQuery>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    engine::require_enabled(&state.db, &actor).await?;
    Ok(Json(match query.acknowledgement_id.as_deref() {
        Some(card) => review::authored_preview_for_card(&state, &actor, &agent, &id, card).await?,
        None => review::authored_preview(&state, &actor, &agent, &id).await?,
    }))
}

/// Withheld decisions use one message; the card shows the reason.
fn withheld() -> AppError {
    AppError::Conflict(
        "This confirmation cannot publish right now; see the card for the reason".into(),
    )
}

pub(crate) async fn decide_authored(
    state: &AppState,
    auth: &AuthUser,
    mut card: crate::models::assistant_acknowledgement::AssistantAcknowledgement,
    allow: bool,
) -> AppResult<crate::models::assistant_acknowledgement::AssistantAcknowledgement> {
    super::login_client_context::require_first_party_human(auth)?;
    let actor = auth.user_id.to_string();
    let reference = card
        .authored_skill
        .clone()
        .ok_or_else(|| AppError::NotFound("Skill review not found".into()))?;
    let chat = crate::services::assistant_skill_authoring::card_authority(&state.db, &actor, &card)
        .await?;
    let actions = review::authored_actions(state, &actor, &card).await?;
    if card.status == "denied" || (allow && actions.state == "pinned" && card.status == "used") {
        return Ok(card);
    }
    let deny_proposal = actions.contains(review::AuthoredAction::Deny);
    let permitted = if allow {
        [
            review::AuthoredAction::Publish,
            review::AuthoredAction::Retry,
            review::AuthoredAction::Check,
        ]
        .into_iter()
        .any(|action| actions.contains(action))
    } else {
        deny_proposal || actions.contains(review::AuthoredAction::DismissCard)
    };
    if !permitted {
        review::audit_withheld_decision(&state.db, &actor, &card, &actions, allow).await;
        return Err(withheld());
    }
    if card.status == "pending" {
        card = acks::decide(&state.db, &actor, &card.conversation_id, &card.id, allow).await?;
        acks::audit_decision(
            &state.db,
            &crate::services::audit_service::AuditActor::from_auth_user(auth),
            &card,
        )
        .await;
    }
    if !allow {
        if card.status == "denied" && deny_proposal {
            match review::reject(
                state,
                &actor,
                &reference.agent_id,
                &reference.proposal_id,
                reference.revision,
                "rejected",
            )
            .await
            {
                Ok(()) | Err(AppError::Conflict(_)) => {}
                Err(error) => return Err(error),
            }
        }
        return Ok(card);
    }
    if !matches!(card.status.as_str(), "allowed" | "used") {
        review::audit_withheld_decision(&state.db, &actor, &card, &actions, allow).await;
        return Err(withheld());
    }
    // This is the reviewed human publication, not a new agent tool effect.
    // Its consumed card and live chat key fence authority; Ornn uses the
    // approving person's normal signed identity without a second card.
    let reader = super::agent_skills::Reader {
        state,
        person: &actor,
        thread_key: None,
        scopes: None,
        chat: None,
    };
    let attempt = review::publication_attempt(state, &actor, &reference).await?;
    let mut binding_failure_code = None;
    let decision: AppResult<()> = async {
        let binding = review::approval_binding_typed(
            state,
            &actor,
            &reference.agent_id,
            &reference.proposal_id,
            reference.revision,
            reference.skills_revision,
            &reader,
        )
        .await
        .map_err(|failure| {
            binding_failure_code = failure.code;
            failure.error
        })?;
        Box::pin(review::approve(
            state,
            &chat,
            &reference.agent_id,
            &reference.proposal_id,
            &card.id,
            &binding,
            &reader,
        ))
        .await?;
        Ok(())
    }
    .await;
    if let Err(error) = decision {
        let code = if card.expires_at <= chrono::Utc::now() {
            review::PublicationFailureCode::ApprovalExpired
        } else if let Some(code) = binding_failure_code {
            code
        } else if review::target_busy_for(state, &actor, &reference)
            .await
            .unwrap_or(false)
        {
            review::PublicationFailureCode::TargetBusy
        } else {
            review::PublicationFailureCode::NyxidRefused
        };
        let _ =
            review::record_pre_claim_failure(state, &actor, &reference, &card, attempt, code).await;
        return Err(error);
    }
    card.status = "used".into();
    Ok(card)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseTargetBody {
    operation_id: String,
    evidence_ref: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseStaleBody {
    #[serde(default = "default_dry_run")]
    dry_run: bool,
    #[serde(default = "default_min_age_hours")]
    min_age_hours: i64,
    #[serde(default = "default_release_limit")]
    limit: usize,
}
fn default_dry_run() -> bool {
    true
}
fn default_min_age_hours() -> i64 {
    24
}
fn default_release_limit() -> usize {
    200
}

/// Admin-only, audited release of stale legacy publications on absence-only
/// evidence; dry run by default. See "Recovering drafts created before #1828"
/// in docs/AGENT_LEARNING.md.
pub async fn admin_release_stale(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ReleaseStaleBody>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin(&state, &auth).await?;
    Ok(Json(
        review::release_stale(
            &state,
            &crate::services::audit_service::AuditActor::from_auth_user(&auth),
            review::StaleRelease {
                dry_run: body.dry_run,
                min_age_hours: body.min_age_hours,
                limit: body.limit,
            },
            |owner: &str| super::agent_skills::OwnerReader::new(&state, owner),
        )
        .await?,
    ))
}

/// Admin-only, audited operator recovery for a target held by an uncertain
/// publication. See "Operator release of an uncertain target" in
/// docs/AGENT_LEARNING.md for the evidence required first.
pub async fn admin_release_target(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(proposal_id): Path<String>,
    Json(body): Json<ReleaseTargetBody>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    super::admin_helpers::require_admin(&state, &auth).await?;
    Ok(Json(
        review::operator_release_target(
            &state,
            &crate::services::audit_service::AuditActor::from_auth_user(&auth),
            &proposal_id,
            &body.operation_id,
            &body.evidence_ref,
        )
        .await?,
    ))
}
