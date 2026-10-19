//! Explicit token-copy cutover controls. Startup only installs indexes; it never
//! asserts that older credential writers have drained or activates discovery.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
    services::{audit_service, token_copy_admission as admission},
};
use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;

#[derive(serde::Serialize)]
pub(crate) struct ControlReport {
    pending_holder_families: &'static [&'static str],
    discovery: Option<admission::DiscoveryReport>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRequest {
    pub old_writers_drained: bool,
}

pub(crate) async fn status(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<ControlReport>> {
    super::admin_helpers::require_admin_or_operator(&state, &auth, "admin.token_admission.status")
        .await?;
    Ok(Json(ControlReport {
        pending_holder_families: admission::pending_holder_families(),
        discovery: admission::discovery_status(&state.db).await?,
    }))
}

pub(crate) async fn start(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<StartRequest>,
) -> AppResult<Json<Option<admission::DiscoveryReport>>> {
    super::admin_helpers::require_admin(&state, &auth).await?;
    super::login_client_context::require_first_party_human(&auth)?;
    if !body.old_writers_drained {
        return Err(AppError::Conflict(
            "Token admission writer cutover acknowledgement is required".into(),
        ));
    }
    admission::activate_discovery(
        &state.db,
        body.old_writers_drained,
        &audit_service::AuditActor::from_auth_user(&auth),
    )
    .await?;
    Ok(Json(admission::discovery_status(&state.db).await?))
}

pub(crate) async fn advance(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(generation): Path<uuid::Uuid>,
) -> AppResult<Json<Option<admission::DiscoveryReport>>> {
    super::admin_helpers::require_admin(&state, &auth).await?;
    super::login_client_context::require_first_party_human(&auth)?;
    audit_service::log_actor_event(
        state.db.clone(),
        &audit_service::AuditActor::from_auth_user(&auth),
        "token_admission.batch_requested",
        Some(serde_json::json!({"generation":generation})),
    )
    .await?;
    Ok(Json(
        admission::advance_discovery(&state.db, &state.encryption_keys, &generation.to_string())
            .await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::{Document, doc};

    #[tokio::test]
    async fn activation_requires_human_admin_acknowledgement_and_complete_holder_coverage() {
        crate::test_utils::init_test_audit_chain_hmac_key();
        let db =
            crate::test_utils::connect_transaction_test_database("token_admission_control").await;
        let state = crate::test_utils::test_app_state(db.clone());
        let admin = uuid::Uuid::new_v4().to_string();
        let ordinary = uuid::Uuid::new_v4().to_string();
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();
        let roles = crate::services::role_service::get_platform_role_ids(&db)
            .await
            .unwrap();
        for (id, is_admin) in [(&admin, true), (&ordinary, false)] {
            let mut user = crate::test_utils::test_user(id, crate::models::user::UserType::Person);
            if is_admin {
                user.role_ids.push(roles.admin.clone());
            }
            db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
                .insert_one(user)
                .await
                .unwrap();
        }
        assert!(matches!(
            start(
                State(state.clone()),
                crate::test_utils::test_auth_user(&ordinary),
                Json(StartRequest {
                    old_writers_drained: true
                })
            )
            .await,
            Err(AppError::Forbidden(_))
        ));
        let mut delegated = crate::test_utils::test_auth_user(&admin);
        delegated.auth_method = crate::mw::auth::AuthMethod::Delegated;
        assert!(
            start(
                State(state.clone()),
                delegated,
                Json(StartRequest {
                    old_writers_drained: true
                })
            )
            .await
            .is_err()
        );
        assert!(
            start(
                State(state.clone()),
                crate::test_utils::test_auth_user(&admin),
                Json(StartRequest {
                    old_writers_drained: false
                })
            )
            .await
            .is_err()
        );
        assert!(admission::discovery_status(&db).await.unwrap().is_none());
        assert!(matches!(
            start(
                State(state.clone()),
                crate::test_utils::test_auth_user(&admin),
                Json(StartRequest {
                    old_writers_drained: true
                })
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        assert!(admission::discovery_status(&db).await.unwrap().is_none());
        let Json(report) = status(
            State(state.clone()),
            crate::test_utils::test_auth_user(&admin),
        )
        .await
        .unwrap();
        assert!(!report.pending_holder_families.is_empty());
        assert!(report.discovery.is_none());
        assert_eq!(
            db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(doc! {"event_type":"token_admission.activated"})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }
}
