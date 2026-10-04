//! Platform-admin runtime retention controls (never delegated to operators).
use crate::{
    AppState,
    errors::AppResult,
    models::assistant_upload_retention::Policy,
    mw::auth::AuthUser,
    services::{assistant_upload_retention as retention, audit_service::AuditActor},
};
use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct PolicyResponse {
    pending_hours: u32,
    image_days: u32,
    images_delete_after_turn: bool,
    document_days: u32,
    tool_image_days: Option<u32>,
}
impl From<Policy> for PolicyResponse {
    fn from(p: Policy) -> Self {
        Self {
            pending_hours: p.pending_hours,
            image_days: p.image_days,
            images_delete_after_turn: p.images_delete_after_turn,
            document_days: p.document_days,
            tool_image_days: p.tool_image_days,
        }
    }
}
#[derive(Serialize)]
pub struct Response {
    effective: PolicyResponse,
    defaults: PolicyResponse,
    overridden: bool,
    revision: i64,
    updated_at: Option<DateTime<Utc>>,
    replica_revision: i64,
    replica_effective: PolicyResponse,
    replica_refreshed_at: Option<DateTime<Utc>>,
    refresh_seconds: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Update {
    pub pending_hours: u32,
    pub image_days: u32,
    pub images_delete_after_turn: bool,
    pub document_days: u32,
    pub tool_image_days: Option<u32>,
}
async fn response(state: &AppState) -> AppResult<Json<Response>> {
    let settings = retention::load(&state.db).await?;
    let replica = state
        .upload_retention
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    Ok(Json(Response {
        effective: settings.policy.unwrap_or_default().into(),
        defaults: Policy::default().into(),
        overridden: settings.policy.is_some(),
        revision: settings.revision,
        updated_at: settings.updated_at,
        replica_revision: replica.settings.revision,
        replica_effective: replica.settings.policy.unwrap_or_default().into(),
        replica_refreshed_at: replica.refreshed_at,
        refresh_seconds: retention::REFRESH_SECONDS,
    }))
}
pub async fn get(State(state): State<AppState>, auth: AuthUser) -> AppResult<Json<Response>> {
    super::admin_helpers::require_admin(&state, &auth).await?;
    response(&state).await
}
pub async fn put(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<Update>,
) -> AppResult<Json<Response>> {
    super::admin_helpers::require_admin(&state, &auth).await?;
    let policy = Policy {
        pending_hours: input.pending_hours,
        image_days: input.image_days,
        images_delete_after_turn: input.images_delete_after_turn,
        document_days: input.document_days,
        tool_image_days: input.tool_image_days,
    };
    Box::pin(retention::update(
        &state.db,
        &AuditActor::from_auth_user(&auth),
        state.audit_chain_hmac_key.as_slice(),
        Some(policy),
    ))
    .await?;
    retention::refresh(&state.db, &state.upload_retention).await?;
    response(&state).await
}
pub async fn reset(State(state): State<AppState>, auth: AuthUser) -> AppResult<Json<Response>> {
    super::admin_helpers::require_admin(&state, &auth).await?;
    Box::pin(retention::update(
        &state.db,
        &AuditActor::from_auth_user(&auth),
        state.audit_chain_hmac_key.as_slice(),
        None,
    ))
    .await?;
    retention::refresh(&state.db, &state.upload_retention).await?;
    response(&state).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        errors::AppError,
        models::user::{COLLECTION_NAME as USERS, UserType},
        test_utils::{
            connect_transaction_test_database, test_app_state, test_auth_user, test_user,
        },
    };
    use mongodb::bson::{Document, doc};
    async fn fixture(admin: bool, operator: bool) -> (AppState, AuthUser) {
        let db = connect_transaction_test_database("admin_upload_retention").await;
        let id = uuid::Uuid::new_v4().to_string();
        let mut user = test_user(&id, UserType::Person);
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();
        let roles = crate::services::role_service::get_platform_role_ids(&db)
            .await
            .unwrap();
        if admin {
            user.role_ids.push(roles.admin);
        }
        if operator {
            user.role_ids.push(roles.operator);
        }
        db.collection(USERS).insert_one(user).await.unwrap();
        (test_app_state(db), test_auth_user(&id))
    }
    fn input() -> Update {
        Update {
            pending_hours: 2,
            image_days: 3,
            images_delete_after_turn: true,
            document_days: 4,
            tool_image_days: Some(5),
        }
    }
    #[tokio::test]
    async fn upload_retention_admin_crud_and_chained_old_new_audit() {
        let (state, auth) = fixture(true, false).await;
        assert_eq!(
            get(State(state.clone()), auth.clone())
                .await
                .unwrap()
                .0
                .effective
                .pending_hours,
            24
        );
        let saved = put(State(state.clone()), auth.clone(), Json(input()))
            .await
            .unwrap()
            .0;
        assert_eq!(saved.effective.pending_hours, 2);
        assert_eq!(saved.replica_revision, 1);
        assert!(saved.overridden);
        let restored = reset(State(state.clone()), auth.clone()).await.unwrap().0;
        assert!(!restored.overridden);
        assert_eq!(restored.revision, 2);
        let logs: Vec<Document> = {
            use futures::TryStreamExt;
            state
                .db
                .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                .find(doc! {"event_type":"admin_upload_retention_updated"})
                .sort(doc! {"seq":1})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap()
        };
        assert_eq!(logs.len(), 2);
        assert_eq!(
            logs[0].get_str("user_id").unwrap(),
            auth.user_id.to_string()
        );
        assert!(logs[0].get_str("entry_hash").is_ok());
        let data = serde_json::to_value(logs[0].get_document("event_data").unwrap()).unwrap();
        assert_eq!(data["old"]["pending_hours"], 24);
        assert_eq!(data["new"]["pending_hours"], 2);
        let verified = crate::services::audit_chain_service::verify_chain(
            &state.db,
            state.audit_chain_hmac_key.as_slice(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(verified.break_info.is_none());
    }
    #[tokio::test]
    async fn upload_retention_rejects_users_operators_and_invalid_values() {
        for operator in [false, true] {
            let (state, auth) = fixture(false, operator).await;
            assert!(matches!(
                get(State(state.clone()), auth.clone()).await,
                Err(AppError::Forbidden(_))
            ));
            assert!(matches!(
                put(State(state.clone()), auth.clone(), Json(input())).await,
                Err(AppError::Forbidden(_))
            ));
            assert!(matches!(
                reset(State(state.clone()), auth).await,
                Err(AppError::Forbidden(_))
            ));
            assert_eq!(retention::load(&state.db).await.unwrap().revision, 0);
        }
        let (state, auth) = fixture(true, false).await;
        let mut bad = input();
        bad.image_days = 366;
        assert!(matches!(
            put(State(state.clone()), auth, Json(bad)).await,
            Err(AppError::BadRequest(_))
        ));
        assert_eq!(retention::load(&state.db).await.unwrap().revision, 0);
    }
}
