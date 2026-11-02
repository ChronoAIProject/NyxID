//! Owner settings for dedicated BYO WhatsApp native migration.
#[path = "channel_whatsapp_native_rollback.rs"]
pub mod rollback;

use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::channel_whatsapp_native::{CallbackPhase, NativeWhatsapp},
    mw::auth::AuthUser,
    services::{audit_service, feature_flag_service, whatsapp_native as native},
};
use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsRequest {
    pub meta_app_id: String,
    pub waba_id: String,
    pub dedicated_app: bool,
    #[serde(default)]
    pub revision: i64,
}

#[derive(Serialize)]
pub struct SettingsResponse {
    pub enabled: bool,
    pub reason: Option<&'static str>,
    pub meta_app_id: Option<String>,
    pub waba_id: Option<String>,
    pub phone_number_id: String,
    pub dedicated_app: bool,
    pub revision: i64,
    pub callback_phase: CallbackPhase,
    pub callback_error: Option<String>,
}

fn response(
    bot: &crate::models::channel_bot::ChannelBot,
    enabled: bool,
    config: Option<NativeWhatsapp>,
) -> SettingsResponse {
    SettingsResponse {
        enabled,
        reason: native::eligibility(bot).err(),
        meta_app_id: config.as_ref().map(|c| c.meta_app_id.clone()),
        waba_id: config
            .as_ref()
            .map(|c| c.waba_id.clone())
            .or_else(|| bot.app_id.clone()),
        phone_number_id: bot.platform_bot_id.clone(),
        dedicated_app: config.as_ref().is_some_and(|c| c.dedicated_app),
        revision: config.as_ref().map_or(0, |c| c.revision),
        callback_phase: config
            .as_ref()
            .map_or(CallbackPhase::Relay, |c| c.callback.phase),
        callback_error: config.and_then(|c| c.callback.last_error),
    }
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<SettingsResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    let (_, bot) = super::channel_bots::resolve_bot_owner_for_write(&state, &actor, &id).await?;
    let enabled =
        feature_flag_service::personal_flag_enabled(&state.db, &actor, native::FLAG).await?;
    let config = native::load(&state.db, &bot).await?;
    Ok(Json(response(&bot, enabled, config)))
}

pub async fn put(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<SettingsRequest>,
) -> AppResult<Json<SettingsResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    let (_, bot) = super::channel_bots::resolve_bot_owner_for_write(&state, &actor, &id).await?;
    if !feature_flag_service::personal_flag_enabled(&state.db, &actor, native::FLAG).await? {
        return Err(AppError::ValidationError(
            "native_whatsapp_flag_disabled".into(),
        ));
    }
    let config = native::configure(
        &state.db,
        &bot,
        body.meta_app_id.trim(),
        body.waba_id.trim(),
        body.dedicated_app,
        body.revision,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "channel_whatsapp_native_configured",
        Some(serde_json::json!({"bot_id": bot.id, "revision": config.revision})),
    );
    Ok(Json(response(&bot, true, Some(config))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::channel_bot::{COLLECTION_NAME, ChannelBot};
    use mongodb::bson::{self, doc};

    #[tokio::test]
    async fn native_whatsapp_settings_require_owner_and_flag_and_refuse_api_keys() {
        let db = crate::test_utils::connect_transaction_test_database("wa_settings").await;
        let owner = uuid::Uuid::new_v4().to_string();
        let id = uuid::Uuid::new_v4().to_string();
        let bot: ChannelBot = bson::from_document(doc! {
            "_id": &id, "user_id": &owner, "platform": "whatsapp", "label":"test",
            "bot_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![1] },
            "app_secret_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![2] },
            "platform_bot_id":"333", "platform_bot_username":"+15551234567", "webhook_secret_hash":"hash", "status":"active", "is_active":true,
            "webhook_registered":true, "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now(),
        }).unwrap();
        db.collection::<ChannelBot>(COLLECTION_NAME)
            .insert_one(&bot)
            .await
            .unwrap();
        let state = crate::test_utils::test_app_state(db.clone());
        let request = || {
            Json(SettingsRequest {
                meta_app_id: "111".into(),
                waba_id: "222".into(),
                dedicated_app: true,
                revision: 0,
            })
        };
        let err = Box::pin(put(
            State(state.clone()),
            crate::test_utils::test_auth_user(&owner),
            Path(id.clone()),
            request(),
        ))
        .await
        .err()
        .unwrap();
        assert!(
            matches!(err, AppError::ValidationError(ref reason) if reason == "native_whatsapp_flag_disabled")
        );
        let stranger = uuid::Uuid::new_v4().to_string();
        let err = Box::pin(get(
            State(state.clone()),
            crate::test_utils::test_auth_user(&stranger),
            Path(id.clone()),
        ))
        .await
        .err()
        .unwrap();
        assert!(matches!(err, AppError::ChannelBotNotFound(_)));
        let mut auth = crate::test_utils::test_auth_user(&owner);
        auth.auth_method = crate::mw::auth::AuthMethod::ApiKey;
        assert!(
            Box::pin(put(State(state.clone()), auth, Path(id.clone()), request()))
                .await
                .is_err()
        );
        assert_eq!(
            db.collection::<bson::Document>(
                crate::models::channel_whatsapp_native::COLLECTION_NAME
            )
            .count_documents(doc! {})
            .await
            .unwrap(),
            0
        );
        db.collection::<bson::Document>("feature_flag_overrides").insert_one(doc! {
            "_id":uuid::Uuid::new_v4().to_string(), "flag_key":native::FLAG, "target_kind":"user",
            "target_key":&owner, "enabled":true, "created_at":bson::DateTime::now(),
            "updated_at":bson::DateTime::now(), "updated_by":&owner,
        }).await.unwrap();
        let result = Box::pin(put(
            State(state.clone()),
            crate::test_utils::test_auth_user(&owner),
            Path(id.clone()),
            request(),
        ))
        .await
        .unwrap()
        .0;
        assert_eq!(result.meta_app_id.as_deref(), Some("111"));
        assert_eq!(result.waba_id.as_deref(), Some("222"));
        assert_eq!(result.callback_phase, CallbackPhase::Relay);
        db.collection::<ChannelBot>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":&id},
                doc! {"$set":{"credential_source":"platform"}},
            )
            .await
            .unwrap();
        let err = Box::pin(put(
            State(state),
            crate::test_utils::test_auth_user(&owner),
            Path(id),
            request(),
        ))
        .await
        .err()
        .unwrap();
        assert!(
            matches!(err, AppError::ValidationError(ref reason) if reason == "dedicated_byo_app_required")
        );
        db.drop().await.unwrap();
    }
}
