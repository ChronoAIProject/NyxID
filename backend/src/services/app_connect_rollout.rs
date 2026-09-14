//! Platform-admin capability and deployment policy jointly enable app requirements.

use mongodb::bson::doc;

use crate::AppState;
use crate::config::AppConfig;
use crate::errors::{AppError, AppResult};
use crate::models::oauth_client::OauthClient;
use crate::models::platform_settings::{
    AppConnectRollout, COLLECTION_NAME, PLATFORM_SETTINGS_ID, PlatformSettings,
};
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppConnectPolicy {
    pub revision: i64,
    pub rollout: AppConnectRollout,
    pub env_default: AppConnectRollout,
    pub override_value: Option<AppConnectRollout>,
}

impl AppConnectPolicy {
    pub fn from_config(config: &AppConfig) -> Self {
        Self::from_settings(config, &PlatformSettings::empty())
    }

    pub fn from_settings(config: &AppConfig, settings: &PlatformSettings) -> Self {
        Self {
            revision: settings.app_connect_policy_revision,
            rollout: settings
                .app_connect_rollout
                .unwrap_or(config.app_connect_rollout),
            env_default: config.app_connect_rollout,
            override_value: settings.app_connect_rollout,
        }
    }
}

pub async fn load_policy(
    db: &mongodb::Database,
    config: &AppConfig,
) -> AppResult<AppConnectPolicy> {
    let settings = super::platform_settings_service::load_settings(db).await?;
    Ok(AppConnectPolicy::from_settings(config, &settings))
}

pub async fn update_policy(
    db: &mongodb::Database,
    config: &AppConfig,
    rollout: Option<AppConnectRollout>,
) -> AppResult<AppConnectPolicy> {
    let mut update = doc! {
        "$setOnInsert": { "_id": PLATFORM_SETTINGS_ID },
        "$inc": { "app_connect_policy_revision": 1_i64 },
    };
    match rollout {
        Some(value) => {
            let encoded = mongodb::bson::to_bson(&value)
                .map_err(|_| AppError::Internal("Could not encode app connect rollout".into()))?;
            update.insert("$set", doc! { "app_connect_rollout": encoded });
        }
        None => {
            update.insert("$unset", doc! { "app_connect_rollout": "" });
        }
    }
    db.collection::<PlatformSettings>(COLLECTION_NAME)
        .update_one(doc! { "_id": PLATFORM_SETTINGS_ID }, update)
        .upsert(true)
        .await?;
    load_policy(db, config).await
}

pub async fn is_enabled_for(state: &AppState, client: &OauthClient) -> AppResult<bool> {
    if !client.is_active || !client.app_connect_capability_enabled {
        return Ok(false);
    }
    match state.app_connect_policy().rollout {
        AppConnectRollout::Disabled => Ok(false),
        AppConnectRollout::Public => Ok(true),
        AppConnectRollout::Allowlist => {
            let Some(owner) = client.created_by.as_deref() else {
                return Ok(false);
            };
            if !state
                .config
                .app_connect_allowed_org_ids
                .iter()
                .any(|id| id == owner)
            {
                return Ok(false);
            }
            Ok(state
                .db
                .collection::<User>(USERS)
                .find_one(doc! { "_id": owner, "user_type": "org", "is_active": true })
                .await?
                .is_some_and(|user| user.user_type == UserType::Org))
        }
    }
}

/// Called only after platform-admin authorization by the HTTP layer.
pub async fn set_client_capability(
    db: &mongodb::Database,
    client_id: &str,
    enabled: bool,
) -> AppResult<bool> {
    let previous = db.collection::<OauthClient>(crate::models::oauth_client::COLLECTION_NAME)
        .find_one_and_update(doc! { "_id": client_id }, doc! { "$set": {
            "app_connect_capability_enabled": enabled, "updated_at": mongodb::bson::DateTime::now(),
        } }).await?.ok_or_else(|| AppError::NotFound("OAuth client not found".into()))?;
    Ok(previous.app_connect_capability_enabled != enabled)
}
