use mongodb::bson::{Document, doc};

use crate::config::AppConfig;
use crate::errors::AppResult;
use crate::models::platform_settings::{
    COLLECTION_NAME as PLATFORM_SETTINGS, PLATFORM_SETTINGS_ID, PlatformSettings,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrokerPolicy {
    pub revision: i64,
    pub broker_require_sender_constraint: bool,
    pub broker_require_sender_constraint_env_default: bool,
    pub broker_require_sender_constraint_override: Option<bool>,
    pub broker_require_admin_capability: bool,
    pub broker_require_admin_capability_env_default: bool,
    pub broker_require_admin_capability_override: Option<bool>,
}

impl BrokerPolicy {
    pub fn from_config(config: &AppConfig) -> Self {
        Self::resolve(
            config.broker_require_sender_constraint(),
            config.broker_require_admin_capability(),
            &PlatformSettings::empty(),
        )
    }

    pub fn resolve(
        sender_env_default: bool,
        admin_env_default: bool,
        settings: &PlatformSettings,
    ) -> Self {
        Self {
            revision: settings.broker_policy_revision,
            broker_require_sender_constraint: settings
                .broker_require_sender_constraint
                .unwrap_or(sender_env_default),
            broker_require_sender_constraint_env_default: sender_env_default,
            broker_require_sender_constraint_override: settings.broker_require_sender_constraint,
            broker_require_admin_capability: settings
                .broker_require_admin_capability
                .unwrap_or(admin_env_default),
            broker_require_admin_capability_env_default: admin_env_default,
            broker_require_admin_capability_override: settings.broker_require_admin_capability,
        }
    }

    pub fn from_settings(config: &AppConfig, settings: &PlatformSettings) -> Self {
        Self::resolve(
            config.broker_require_sender_constraint(),
            config.broker_require_admin_capability(),
            settings,
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BrokerSettingsPatch {
    pub broker_require_sender_constraint: Option<Option<bool>>,
    pub broker_require_admin_capability: Option<Option<bool>>,
}

pub async fn load_settings(db: &mongodb::Database) -> AppResult<PlatformSettings> {
    let settings = db
        .collection::<PlatformSettings>(PLATFORM_SETTINGS)
        .find_one(doc! { "_id": PLATFORM_SETTINGS_ID })
        .await?
        .unwrap_or_else(PlatformSettings::empty);

    Ok(settings)
}

pub async fn load_broker_policy(
    db: &mongodb::Database,
    config: &AppConfig,
) -> AppResult<BrokerPolicy> {
    let settings = load_settings(db).await?;
    Ok(BrokerPolicy::from_settings(config, &settings))
}

pub async fn update_broker_settings(
    db: &mongodb::Database,
    patch: BrokerSettingsPatch,
) -> AppResult<PlatformSettings> {
    if patch.broker_require_sender_constraint.is_none()
        && patch.broker_require_admin_capability.is_none()
    {
        return load_settings(db).await;
    }

    let mut set_doc = Document::new();
    let mut unset_doc = Document::new();

    apply_optional_bool_patch(
        &mut set_doc,
        &mut unset_doc,
        "broker_require_sender_constraint",
        patch.broker_require_sender_constraint,
    );
    apply_optional_bool_patch(
        &mut set_doc,
        &mut unset_doc,
        "broker_require_admin_capability",
        patch.broker_require_admin_capability,
    );

    let mut update_doc = doc! {
        "$setOnInsert": { "_id": PLATFORM_SETTINGS_ID },
        "$inc": { "broker_policy_revision": 1_i64 },
    };
    if !set_doc.is_empty() {
        update_doc.insert("$set", set_doc);
    }
    if !unset_doc.is_empty() {
        update_doc.insert("$unset", unset_doc);
    }

    db.collection::<PlatformSettings>(PLATFORM_SETTINGS)
        .update_one(doc! { "_id": PLATFORM_SETTINGS_ID }, update_doc)
        .upsert(true)
        .await?;

    load_settings(db).await
}

fn apply_optional_bool_patch(
    set_doc: &mut Document,
    unset_doc: &mut Document,
    field: &str,
    patch: Option<Option<bool>>,
) {
    match patch {
        Some(Some(value)) => {
            set_doc.insert(field, value);
        }
        Some(None) => {
            unset_doc.insert(field, "");
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broker_policy_uses_env_defaults_when_overrides_absent() {
        let settings = PlatformSettings::empty();
        let policy = BrokerPolicy::resolve(true, false, &settings);

        assert!(policy.broker_require_sender_constraint);
        assert!(!policy.broker_require_admin_capability);
        assert_eq!(policy.revision, 0);
        assert_eq!(policy.broker_require_sender_constraint_override, None);
        assert_eq!(policy.broker_require_admin_capability_override, None);
    }

    #[test]
    fn broker_policy_db_override_wins_over_env_default() {
        let settings = PlatformSettings {
            id: PLATFORM_SETTINGS_ID.to_string(),
            broker_require_sender_constraint: Some(false),
            broker_require_admin_capability: Some(true),
            broker_policy_revision: 3,
        };
        let policy = BrokerPolicy::resolve(true, false, &settings);

        assert_eq!(policy.revision, 3);
        assert!(!policy.broker_require_sender_constraint);
        assert!(policy.broker_require_admin_capability);
        assert_eq!(
            policy.broker_require_sender_constraint_override,
            Some(false)
        );
        assert_eq!(policy.broker_require_admin_capability_override, Some(true));
    }
}
