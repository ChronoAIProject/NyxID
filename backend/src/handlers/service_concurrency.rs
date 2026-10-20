use crate::{
    AppState,
    errors::AppResult,
    models::service_concurrency::{ConcurrencyOverride, ServiceConcurrencyPolicy},
    mw::auth::AuthUser,
    services::{audit_service, service_concurrency_service},
};
use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};

/// Dedicated admin DTOs: catalog/connection APIs never expose override targets.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Override {
    pub id: String,
    pub limit: Option<u32>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub default_limit: Option<u32>,
    #[serde(default)]
    pub users: Vec<Override>,
    #[serde(default)]
    pub orgs: Vec<Override>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyResponse {
    pub policy: Option<Policy>,
}
impl From<ServiceConcurrencyPolicy> for Policy {
    fn from(p: ServiceConcurrencyPolicy) -> Self {
        let map = |v: ConcurrencyOverride| Override {
            id: v.id,
            limit: v.limit,
        };
        Self {
            default_limit: p.default_limit,
            users: p.users.into_iter().map(map).collect(),
            orgs: p.orgs.into_iter().map(map).collect(),
        }
    }
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<PolicyResponse>> {
    super::admin_helpers::require_admin(&state, &auth).await?;
    let service = super::services_helpers::fetch_service(&state, &id).await?;
    Ok(Json(PolicyResponse {
        policy: service.concurrency_policy.map(Into::into),
    }))
}

pub async fn put(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<PolicyResponse>,
) -> AppResult<Json<PolicyResponse>> {
    super::admin_helpers::require_admin(&state, &auth).await?;
    let policy = input.policy.map(|p| {
        let map = |v: Override| ConcurrencyOverride {
            id: v.id,
            limit: v.limit,
        };
        ServiceConcurrencyPolicy {
            service_id: id.clone(),
            default_limit: p.default_limit,
            users: p.users.into_iter().map(map).collect(),
            orgs: p.orgs.into_iter().map(map).collect(),
        }
    });
    service_concurrency_service::set_policy(&state.db, &id, policy.as_ref()).await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "service_concurrency_policy_updated",
        Some(serde_json::json!({
            "service_id":id,"enabled":policy.is_some(),"default_limit":policy.as_ref().and_then(|p|p.default_limit),
            "user_override_count":policy.as_ref().map_or(0,|p|p.users.len()),"org_override_count":policy.as_ref().map_or(0,|p|p.orgs.len())
        })),
    );
    Ok(Json(PolicyResponse {
        policy: policy.map(Into::into),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        errors::AppError,
        models::user::{COLLECTION_NAME as USERS, UserType},
        test_utils::*,
    };
    #[tokio::test]
    async fn admin_only_api_and_policy_is_additive() {
        let db = connect_transaction_test_database("concurrency_admin").await;
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();
        let roles = crate::services::role_service::get_platform_role_ids(&db)
            .await
            .unwrap();
        let actor = uuid::Uuid::new_v4().to_string();
        let user = test_user(&actor, UserType::Person);
        db.collection(USERS).insert_one(user).await.unwrap();
        let svc = test_auto_connected_catalog_service();
        db.collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&svc)
        .await
        .unwrap();
        let state = test_app_state(db.clone());
        let auth = test_auth_user(&actor);
        assert!(matches!(
            get(State(state.clone()), auth.clone(), Path(svc.id.clone())).await,
            Err(AppError::Forbidden(_))
        ));
        assert!(matches!(
            put(
                State(state.clone()),
                auth.clone(),
                Path(svc.id.clone()),
                Json(PolicyResponse { policy: None })
            )
            .await,
            Err(AppError::Forbidden(_))
        ));
        db.collection::<mongodb::bson::Document>(USERS)
            .update_one(
                mongodb::bson::doc! {"_id":&actor},
                mongodb::bson::doc! {"$set":{"role_ids":[roles.admin],"is_admin":true}},
            )
            .await
            .unwrap();
        let input = PolicyResponse {
            policy: Some(Policy {
                default_limit: Some(2),
                users: vec![Override {
                    id: actor.clone(),
                    limit: None,
                }],
                orgs: vec![],
            }),
        };
        let _ = put(
            State(state.clone()),
            auth.clone(),
            Path(svc.id.clone()),
            Json(input),
        )
        .await
        .unwrap();
        let result = get(State(state.clone()), auth.clone(), Path(svc.id.clone()))
            .await
            .unwrap();
        assert_eq!(result.policy.as_ref().unwrap().default_limit, Some(2));
        assert!(result.policy.as_ref().unwrap().users[0].limit.is_none());
        let stored = super::super::services_helpers::fetch_service(&state, &svc.id)
            .await
            .unwrap();
        assert_eq!(stored.concurrency_policy.unwrap().service_id, svc.id);
        assert_eq!(stored.base_url, svc.base_url);
        let _ = put(
            State(state.clone()),
            auth.clone(),
            Path(svc.id.clone()),
            Json(PolicyResponse { policy: None }),
        )
        .await
        .unwrap();
        assert!(
            get(State(state), auth, Path(svc.id))
                .await
                .unwrap()
                .policy
                .is_none()
        );
    }
}
