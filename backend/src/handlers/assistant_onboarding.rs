//! First-run NyxBot onboarding: interest answers and next-step recommendations.
//! Human-only (first-party sessions), like the rest of `/assistant/nyxagent`.
use axum::{
    Json,
    extract::{Path, Query, State},
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
    /// New clients resolve live provider choices before starting this suggestion.
    connect: bool,
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
            connect: !crate::services::assistant_onboarding_connect::candidates(rec.id).is_empty(),
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

#[derive(Serialize)]
pub struct ConnectChoiceResponse {
    slug: String,
    name: String,
    scopes: Vec<String>,
}

#[derive(Serialize)]
pub struct SuggestionSetupResponse {
    connected: bool,
    choices: Vec<ConnectChoiceResponse>,
    prompt: String,
}

/// Resolve an owner-clicked setup using current catalog and account state.
pub async fn suggestion_setup(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<SuggestionSetupResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let setup =
        crate::services::assistant_onboarding_connect::prepare(&state.db, &owner, &id).await?;
    Ok(Json(SuggestionSetupResponse {
        connected: setup.connected,
        prompt: setup.prompt,
        choices: setup
            .choices
            .into_iter()
            .map(|c| ConnectChoiceResponse {
                slug: c.slug,
                name: c.name,
                scopes: c.scopes,
            })
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        mw::auth::AuthMethod,
        services::feature_flag_service::{self, FlagTarget},
    };

    #[tokio::test]
    async fn suggestion_setup_is_human_only_flag_gated_and_metadata_only() {
        let db =
            crate::test_utils::connect_transaction_test_database("suggestion_setup_http").await;
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection("users")
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let state = crate::test_utils::test_app_state(db);
        let auth = crate::test_utils::test_auth_user(&owner);
        feature_flag_service::set_platform_override(
            &state.db,
            feature_flag_service::NYXAGENT_ENGINE_FLAG_KEY,
            &FlagTarget::Global,
            true,
            "admin",
        )
        .await
        .unwrap();
        let Json(setup) = suggestion_setup(
            State(state.clone()),
            auth.clone(),
            Path("connect_calendar".into()),
        )
        .await
        .unwrap();
        let body = serde_json::to_value(setup).unwrap();
        assert_eq!(body["connected"], false);
        assert_eq!(body["choices"], serde_json::json!([]));
        assert!(body.get("connect_url").is_none());
        assert_eq!(
            state
                .db
                .collection::<mongodb::bson::Document>(crate::models::connect_link::COLLECTION_NAME)
                .count_documents(mongodb::bson::doc! {})
                .await
                .unwrap(),
            0
        );
        for method in [
            AuthMethod::ApiKey,
            AuthMethod::Delegated,
            AuthMethod::Relay,
            AuthMethod::ServiceAccount,
        ] {
            let mut restricted = auth.clone();
            restricted.auth_method = method;
            assert!(
                suggestion_setup(
                    State(state.clone()),
                    restricted,
                    Path("connect_calendar".into())
                )
                .await
                .is_err()
            );
        }
        let mut oauth = auth.clone();
        oauth.oauth_client_id = Some("third-party".into());
        assert!(
            suggestion_setup(State(state.clone()), oauth, Path("connect_calendar".into()))
                .await
                .is_err()
        );
        feature_flag_service::set_platform_override(
            &state.db,
            feature_flag_service::NYXAGENT_ENGINE_FLAG_KEY,
            &FlagTarget::Global,
            false,
            "admin",
        )
        .await
        .unwrap();
        assert!(matches!(
            suggestion_setup(State(state), auth, Path("connect_calendar".into())).await,
            Err(crate::errors::AppError::NotFound(_))
        ));
    }
}
