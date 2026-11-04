//! First-run NyxBot onboarding: interest answers and next-step recommendations.
//! Human-only (first-party sessions), like the rest of `/assistant/nyxagent`.
use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    errors::AppResult,
    models::assistant_onboarding::{
        AssistantOnboarding, INTRO_VERSION, InterestTopic, OnboardingOutcome, WorkStyle,
    },
    mw::auth::AuthUser,
    services::{
        assistant_nyxagent as engine, assistant_onboarding_service as onboarding, audit_service,
    },
};

#[derive(Serialize)]
pub struct ChoiceResponse {
    id: serde_json::Value,
    label: &'static str,
    icon: &'static str,
    /// Topics only: the work styles a topic is offered to.
    #[serde(skip_serializing_if = "Option::is_none")]
    work_styles: Option<&'static [WorkStyle]>,
}

#[derive(Serialize)]
pub struct ProfileResponse {
    outcome: OnboardingOutcome,
    topics: Vec<InterestTopic>,
    work_style: Option<WorkStyle>,
    intro_version: i32,
    updated_at: String,
}

#[derive(Serialize)]
pub struct RecommendationResponse {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    prompt: &'static str,
    icon: &'static str,
}

#[derive(Serialize)]
pub struct OnboardingResponse {
    /// Open the first-run intro now.
    show_intro: bool,
    /// The owner hid the home's suggestions (the intro still offers them).
    suggestions_hidden: bool,
    intro_version: i32,
    profile: Option<ProfileResponse>,
    topics: Vec<ChoiceResponse>,
    work_styles: Vec<ChoiceResponse>,
    recommendations: Vec<RecommendationResponse>,
}

#[derive(Deserialize)]
pub struct OnboardingQuery {
    limit: Option<usize>,
}

fn choice<T: Serialize>(id: T, (label, icon): (&'static str, &'static str)) -> ChoiceResponse {
    ChoiceResponse {
        id: serde_json::to_value(id).unwrap_or_default(),
        label,
        icon,
        work_styles: None,
    }
}

async fn respond(
    state: &AppState,
    owner: &str,
    profile: Option<AssistantOnboarding>,
    limit: usize,
) -> AppResult<OnboardingResponse> {
    let (show_intro, signals) = tokio::try_join!(
        onboarding::should_show_intro(&state.db, owner, profile.as_ref()),
        onboarding::load_signals(&state.db, owner),
    )?;
    let recommendations = onboarding::recommend(profile.as_ref(), &signals, limit)
        .into_iter()
        .map(|rec| RecommendationResponse {
            id: rec.id,
            title: rec.title,
            description: rec.description,
            prompt: rec.prompt,
            icon: rec.icon,
        })
        .collect();
    let suggestions_hidden = profile
        .as_ref()
        .is_some_and(|row| row.suggestions_hidden_at.is_some());
    Ok(OnboardingResponse {
        show_intro,
        suggestions_hidden,
        intro_version: INTRO_VERSION,
        profile: profile.map(|row| ProfileResponse {
            outcome: row.outcome,
            topics: row
                .topics
                .into_iter()
                .filter(|topic| *topic != InterestTopic::Unsupported)
                .collect(),
            work_style: row
                .work_style
                .filter(|style| *style != WorkStyle::Unsupported),
            intro_version: row.intro_version,
            updated_at: row
                .updated_at
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        }),
        topics: onboarding::TOPICS
            .iter()
            .map(|topic| ChoiceResponse {
                work_styles: Some(onboarding::topic_styles(*topic)),
                ..choice(topic, onboarding::topic_label(*topic))
            })
            .collect(),
        work_styles: onboarding::WORK_STYLES
            .iter()
            .map(|style| choice(style, onboarding::work_style_label(*style)))
            .collect(),
        recommendations,
    })
}

/// GET /api/v1/assistant/nyxagent/onboarding
pub async fn get_onboarding(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<OnboardingQuery>,
) -> AppResult<Json<OnboardingResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let profile = onboarding::get(&state.db, &owner).await?;
    let limit = query.limit.unwrap_or(onboarding::DEFAULT_RECOMMENDATIONS);
    Ok(Json(respond(&state, &owner, profile, limit).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveOnboardingRequest {
    outcome: OnboardingOutcome,
    #[serde(default)]
    topics: Vec<InterestTopic>,
    #[serde(default)]
    work_style: Option<WorkStyle>,
}

/// PUT /api/v1/assistant/nyxagent/onboarding
///
/// Finish or skip the intro, replacing earlier answers. NyxBot's onboarding
/// memory note follows the answers (removed when none were given).
pub async fn save_onboarding(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SaveOnboardingRequest>,
) -> AppResult<Json<OnboardingResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    if body.topics.len() > onboarding::TOPICS.len() * 2 {
        return Err(crate::errors::AppError::ValidationError(
            "Too many onboarding topics".into(),
        ));
    }
    let row = onboarding::save(
        &state.db,
        &owner,
        onboarding::Answers {
            outcome: body.outcome,
            topics: body.topics,
            work_style: body.work_style,
        },
    )
    .await?;
    let _ = audit_service::log_actor_event(
        state.db.clone(),
        &audit_service::AuditActor::from_auth_user(&auth),
        "assistant_onboarding_saved",
        Some(serde_json::json!({
            "outcome": row.outcome,
            "topic_count": row.topics.len(),
            "work_style_set": row.work_style.is_some(),
            "intro_version": row.intro_version,
        })),
    )
    .await;
    Ok(Json(
        respond(
            &state,
            &owner,
            Some(row),
            onboarding::DEFAULT_RECOMMENDATIONS,
        )
        .await?,
    ))
}

/// POST /api/v1/assistant/nyxagent/onboarding/suggestions/hide
///
/// Hides the home's suggestions until the intro is retaken; returns the
/// refreshed state.
pub async fn hide_suggestions(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<OnboardingResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    onboarding::hide_suggestions(&state.db, &owner).await?;
    let profile = onboarding::get(&state.db, &owner).await?;
    Ok(Json(
        respond(&state, &owner, profile, onboarding::DEFAULT_RECOMMENDATIONS).await?,
    ))
}
