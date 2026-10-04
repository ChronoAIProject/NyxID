use std::collections::HashMap;

use futures::TryStreamExt;
use mongodb::{Database, bson::doc};
use serde::Deserialize;

use crate::errors::AppResult;
use crate::models::user_api_key::UserApiKey;
use crate::models::user_provider_token::COLLECTION_NAME;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OAuthAppSource {
    Platform,
    Byo,
}

impl OAuthAppSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Platform => "platform",
            Self::Byo => "byo",
        }
    }
}

/// No credential decryption or provider call is needed to identify the app.
/// Modern connection refresh uses embedded BYO credentials, otherwise the
/// provider's app. Legacy keys refresh through their provider-token record.
pub fn from_key(key: &UserApiKey) -> Option<OAuthAppSource> {
    if !matches!(key.credential_type.as_str(), "oauth2" | "device_code") {
        return None;
    }
    match key.credential_source.as_deref() {
        Some("platform") => Some(OAuthAppSource::Platform),
        Some("byo") => Some(OAuthAppSource::Byo),
        Some(_) => None,
        None if key.connection_id.is_some() && key.provider_config_id.is_some() => {
            Some(if key.user_oauth_client_id_encrypted.is_some() {
                OAuthAppSource::Byo
            } else {
                OAuthAppSource::Platform
            })
        }
        None => None,
    }
}

#[derive(Deserialize)]
struct LegacyAppMetadata {
    #[serde(rename = "_id")]
    id: String,
    user_id: String,
    provider_config_id: String,
    #[serde(default)]
    credential_user_id: Option<String>,
}

fn needs_legacy_lookup(key: &UserApiKey) -> bool {
    matches!(key.credential_type.as_str(), "oauth2" | "device_code")
        && key.credential_source.is_none()
        && key.connection_id.is_none()
        && key.provider_config_id.is_some()
}

fn from_legacy(key: &UserApiKey, tokens: &[LegacyAppMetadata]) -> Option<OAuthAppSource> {
    let provider_id = key.provider_config_id.as_deref()?;
    let source_id = (key.source.as_deref() == Some("migration_provider_token"))
        .then_some(key.source_id.as_deref())
        .flatten();
    let mut matches = tokens.iter().filter(|token| {
        token.user_id == key.user_id
            && token.provider_config_id == provider_id
            && source_id.is_none_or(|id| token.id == id)
    });
    let token = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(if token.credential_user_id.is_some() {
        OAuthAppSource::Byo
    } else {
        OAuthAppSource::Platform
    })
}

/// Request-scoped projection for already-authorized credential records. Only
/// safe owner/provider/app-source metadata is loaded for unresolved legacy keys.
pub async fn load(
    db: &Database,
    keys: &[&UserApiKey],
) -> AppResult<HashMap<String, OAuthAppSource>> {
    let filters: Vec<_> = keys
        .iter()
        .filter(|key| needs_legacy_lookup(key))
        .map(|key| {
            let mut filter = doc! {
                "user_id": &key.user_id,
                "provider_config_id": key.provider_config_id.as_deref().unwrap(),
            };
            if key.source.as_deref() == Some("migration_provider_token")
                && let Some(id) = &key.source_id
            {
                filter.insert("_id", id);
            }
            filter
        })
        .collect();
    let tokens: Vec<LegacyAppMetadata> = if filters.is_empty() {
        Vec::new()
    } else {
        db.collection::<LegacyAppMetadata>(COLLECTION_NAME)
            .find(doc! { "$or": filters })
            .projection(doc! {
                "_id": 1, "user_id": 1, "provider_config_id": 1,
                "credential_user_id": 1,
            })
            .await?
            .try_collect()
            .await?
    };
    Ok(keys
        .iter()
        .filter_map(|key| {
            from_key(key)
                .or_else(|| {
                    needs_legacy_lookup(key)
                        .then(|| from_legacy(key, &tokens))
                        .flatten()
                })
                .map(|source| (key.id.clone(), source))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> UserApiKey {
        mongodb::bson::from_document(doc! {
            "_id": "key", "user_id": "person", "label": "X",
            "credential_type": "oauth2", "status": "active",
            "provider_config_id": "x", "connection_id": "connection",
            "created_at": mongodb::bson::DateTime::now(),
            "updated_at": mongodb::bson::DateTime::now(),
        })
        .unwrap()
    }

    fn token(id: &str, owner: &str, provider: &str, supplied: bool) -> LegacyAppMetadata {
        LegacyAppMetadata {
            id: id.into(),
            user_id: owner.into(),
            provider_config_id: provider.into(),
            credential_user_id: supplied.then(|| owner.into()),
        }
    }

    #[test]
    fn oauth_app_metadata_resolves_modern_keys_without_new_marker() {
        let mut key = key();
        assert_eq!(from_key(&key), Some(OAuthAppSource::Platform));
        key.user_oauth_client_id_encrypted = Some(vec![1]);
        assert_eq!(from_key(&key), Some(OAuthAppSource::Byo));
        key.credential_source = Some("platform".into());
        assert_eq!(from_key(&key), Some(OAuthAppSource::Platform));
        key.credential_source = Some("unsupported".into());
        assert_eq!(from_key(&key), None);
    }

    #[test]
    fn oauth_app_metadata_legacy_uses_exact_owner_provider_and_migration_source() {
        let mut key = key();
        key.connection_id = None;
        key.status = "revoked".into();
        key.source = Some("migration_provider_token".into());
        key.source_id = Some("original".into());
        // A retained unrelated BYO app must not change the original token's app.
        key.user_oauth_client_id_encrypted = Some(vec![1]);
        let tokens = [
            token("original", "person", "x", false),
            token("other", "person", "x", true),
            token("original", "org", "x", true),
            token("original", "person", "other-provider", true),
        ];
        assert_eq!(from_key(&key), None);
        assert_eq!(from_legacy(&key, &tokens), Some(OAuthAppSource::Platform));
        key.source_id = Some("other".into());
        assert_eq!(from_legacy(&key, &tokens), Some(OAuthAppSource::Byo));
        key.source_id = Some("missing".into());
        assert_eq!(from_legacy(&key, &tokens), None);
    }

    #[test]
    fn oauth_app_metadata_does_not_guess_from_absent_or_ambiguous_legacy_tokens() {
        let mut key = key();
        key.connection_id = None;
        assert!(needs_legacy_lookup(&key));
        assert_eq!(from_legacy(&key, &[]), None);
        let tokens = [
            token("a", "person", "x", false),
            token("b", "person", "x", true),
        ];
        assert_eq!(from_legacy(&key, &tokens), None);
        assert_eq!(
            from_legacy(&key, &tokens[..1]),
            Some(OAuthAppSource::Platform)
        );
        key.credential_type = "api_key".into();
        assert!(!needs_legacy_lookup(&key));
        assert_eq!(from_key(&key), None);
    }
}
