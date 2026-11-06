use crate::{
    AppState,
    errors::AppResult,
    models::platform_settings::UtilityInference,
    mw::auth::AuthUser,
    services::{audit_service, utility_inference_service as utility},
};
use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UtilityRoute {
    pub service_slug: String,
    pub model: String,
}

#[derive(Serialize)]
pub struct UtilityResponse {
    pub service_slug: String,
    pub model: String,
}

/// GET /api/v1/admin/settings/utility-inference; null means legacy selection.
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Option<UtilityResponse>>> {
    super::admin_helpers::require_admin(&state, &auth).await?;
    Ok(Json(utility::load(&state.db).await?.map(|config| {
        UtilityResponse {
            service_slug: config.service_slug,
            model: config.model,
        }
    })))
}

/// PUT the complete pair, or JSON null to clear it permanently across restarts.
pub async fn put(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<Option<UtilityRoute>>,
) -> AppResult<Json<Option<UtilityResponse>>> {
    super::admin_helpers::require_admin(&state, &auth).await?;
    utility::set(
        &state.db,
        body.map(|config| UtilityInference {
            service_slug: config.service_slug,
            model: config.model,
        }),
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "admin_utility_inference_updated",
        None,
    );
    get(State(state), auth).await
}

pub async fn get_crm(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Option<UtilityResponse>>> {
    super::admin_helpers::require_admin(&state, &auth).await?;
    Ok(Json(
        crate::services::platform_settings_service::load_settings(&state.db)
            .await?
            .crm_inference
            .map(|c| UtilityResponse {
                service_slug: c.service_slug,
                model: c.model,
            }),
    ))
}

pub async fn put_crm(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<Option<UtilityRoute>>,
) -> AppResult<Json<Option<UtilityResponse>>> {
    super::admin_helpers::require_admin(&state, &auth).await?;
    utility::set_route(
        &state.db,
        body.map(|c| UtilityInference {
            service_slug: c.service_slug,
            model: c.model,
        }),
        utility::InferenceRoute::Crm,
    )
    .await?;
    audit_service::log_for_user(state.db.clone(), &auth, "admin_crm_inference_updated", None);
    get_crm(State(state), auth).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::assistant_oneshot_inference::tests::{ACTOR, utility_fixture};
    use mongodb::bson::{Document, doc};

    #[tokio::test]
    async fn utility_inference_admin_api_requires_admin_and_roundtrips_clear() {
        let (state, _, _) = utility_fixture().await;
        crate::services::role_service::seed_system_roles(&state.db)
            .await
            .unwrap();
        let roles = crate::services::role_service::get_platform_role_ids(&state.db)
            .await
            .unwrap();
        let auth = crate::test_utils::test_auth_user(ACTOR);
        assert!(get(State(state.clone()), auth.clone()).await.is_err());
        assert!(
            put(State(state.clone()), auth.clone(), Json(None))
                .await
                .is_err()
        );
        state
            .db
            .collection::<Document>("users")
            .update_one(doc! {"_id":ACTOR}, doc! {"$set":{"role_ids":[roles.admin]}})
            .await
            .unwrap();
        let result = put(
            State(state.clone()),
            auth.clone(),
            Json(Some(UtilityRoute {
                service_slug: "chrono-llm-public".into(),
                model: "gpt-4.1-mini".into(),
            })),
        )
        .await
        .unwrap();
        assert_eq!(result.0.unwrap().model, "gpt-4.1-mini");
        assert!(
            put(State(state.clone()), auth.clone(), Json(None))
                .await
                .unwrap()
                .0
                .is_none()
        );
        utility::seed(&state.db).await.unwrap();
        assert!(get(State(state), auth).await.unwrap().0.is_none());
    }
}

#[cfg(test)]
mod crm_tests {
    use super::*;
    use crate::services::assistant_oneshot_inference::tests::{ACTOR, utility_fixture};
    use mongodb::bson::{Document, doc};
    #[tokio::test]
    async fn crm_inference_admin_authoring_validates_platform_route_and_persists_clear() {
        let (state, _, _) = utility_fixture().await;
        let auth = crate::test_utils::test_auth_user(ACTOR);
        assert!(get_crm(State(state.clone()), auth.clone()).await.is_err());
        assert!(
            put_crm(State(state.clone()), auth.clone(), Json(None))
                .await
                .is_err()
        );
        crate::services::role_service::seed_system_roles(&state.db)
            .await
            .unwrap();
        let roles = crate::services::role_service::get_platform_role_ids(&state.db)
            .await
            .unwrap();
        state
            .db
            .collection::<Document>("users")
            .update_one(doc! {"_id":ACTOR}, doc! {"$set":{"role_ids":[roles.admin]}})
            .await
            .unwrap();
        assert!(
            put_crm(
                State(state.clone()),
                auth.clone(),
                Json(Some(UtilityRoute {
                    service_slug: "not-a-route".into(),
                    model: "gpt-6-luna".into()
                }))
            )
            .await
            .is_err()
        );
        let saved = put_crm(
            State(state.clone()),
            auth.clone(),
            Json(Some(UtilityRoute {
                service_slug: "chrono-llm-public".into(),
                model: "gpt-6-luna".into(),
            })),
        )
        .await
        .unwrap()
        .0
        .unwrap();
        assert_eq!(saved.model, "gpt-6-luna");
        assert!(
            put_crm(State(state.clone()), auth.clone(), Json(None))
                .await
                .unwrap()
                .0
                .is_none()
        );
        utility::seed(&state.db).await.unwrap();
        assert!(
            get_crm(State(state.clone()), auth)
                .await
                .unwrap()
                .0
                .is_none()
        );
        assert!(utility::load(&state.db).await.unwrap().is_some());
        state.db.drop().await.unwrap();
    }
}
