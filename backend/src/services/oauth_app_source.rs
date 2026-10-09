use std::collections::HashMap;

use futures::TryStreamExt;
use mongodb::{Database, bson::doc};
use serde::Deserialize;

use crate::errors::AppResult;
use crate::models::user_api_key::{OAuthAppObservation, UserApiKey};
use crate::models::user_provider_token::COLLECTION_NAME;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OAuthAppSource {
    Platform,
    Byo,
}

impl OAuthAppSource {
    pub fn from_credential_owner(owner: Option<&str>) -> Self {
        if owner.is_some() {
            Self::Byo
        } else {
            Self::Platform
        }
    }

    pub fn observation(self, credential_epoch: i64) -> OAuthAppObservation {
        OAuthAppObservation {
            source: self.as_str().into(),
            credential_epoch,
            observed_at: chrono::Utc::now(),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Platform => "platform",
            Self::Byo => "byo",
        }
    }
}

/// App selection or a successful exchange is evidence; a connection ID,
/// retained developer app, or unexpired token alone is not.
pub fn from_key(key: &UserApiKey) -> Option<OAuthAppSource> {
    if !matches!(key.credential_type.as_str(), "oauth2" | "device_code") {
        return None;
    }
    let observed = key
        .oauth_app_observation
        .as_ref()
        .filter(|observation| {
            has_token_material(key)
                && observation.credential_epoch == key.credential_epoch
                && key
                    .last_authorized_at
                    .is_none_or(|authorized| observation.observed_at >= authorized)
        })
        .and_then(|observation| parse_source(&observation.source));
    match key.credential_source.as_deref() {
        Some(source) if observed.is_some() && parse_source(source) != observed => None,
        Some("platform") => Some(OAuthAppSource::Platform),
        Some("byo") => Some(OAuthAppSource::Byo),
        Some(_) => None,
        None => observed,
    }
}

fn parse_source(source: &str) -> Option<OAuthAppSource> {
    match source {
        "platform" => Some(OAuthAppSource::Platform),
        "byo" => Some(OAuthAppSource::Byo),
        _ => None,
    }
}

fn has_token_material(key: &UserApiKey) -> bool {
    key.access_token_encrypted
        .as_ref()
        .is_some_and(|token| !token.is_empty())
        || key
            .refresh_token_encrypted
            .as_ref()
            .is_some_and(|token| !token.is_empty())
}

#[derive(Deserialize)]
struct LegacyAppMetadata {
    #[serde(rename = "_id")]
    id: String,
    user_id: String,
    provider_config_id: String,
    #[serde(default)]
    credential_user_id: Option<String>,
    #[serde(default)]
    connection_id: Option<String>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    access_token_encrypted: Option<Vec<u8>>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    refresh_token_encrypted: Option<Vec<u8>>,
}

fn needs_legacy_lookup(key: &UserApiKey) -> bool {
    matches!(key.credential_type.as_str(), "oauth2" | "device_code")
        && key.credential_source.is_none()
        && key.provider_config_id.is_some()
        && from_key(key).is_none()
        && has_token_material(key)
}

fn from_legacy(key: &UserApiKey, tokens: &[LegacyAppMetadata]) -> Option<OAuthAppSource> {
    let provider_id = key.provider_config_id.as_deref()?;
    let source_id = matches!(
        key.source.as_deref(),
        Some("migration_provider_token" | "user_created")
    )
    .then_some(key.source_id.as_deref())
    .flatten();
    let mut matches = tokens.iter().filter(|token| {
        token.user_id == key.user_id
            && token.provider_config_id == provider_id
            && source_id.is_none_or(|id| token.id == id)
            && (key.connection_id.is_none()
                || source_id.is_some()
                || key.connection_id == token.connection_id)
            // Migration and legacy sync copy ciphertext unchanged. Matching
            // both token fields ties the source to this credential revision.
            && key.access_token_encrypted == token.access_token_encrypted
            && key.refresh_token_encrypted == token.refresh_token_encrypted
            && has_token_material(key)
    });
    let token = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(OAuthAppSource::from_credential_owner(
        token.credential_user_id.as_deref(),
    ))
}

/// Request-scoped projection for already-authorized credential records. Only
/// legacy token copies are matched without decrypting them. Ciphertext is kept
/// inside this resolver; only the source enum is returned.
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
            if matches!(
                key.source.as_deref(),
                Some("migration_provider_token" | "user_created")
            ) && let Some(id) = &key.source_id
            {
                filter.insert("_id", id);
            } else if let Some(connection_id) = &key.connection_id {
                filter.insert("connection_id", connection_id);
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
                "credential_user_id": 1, "connection_id": 1,
                "access_token_encrypted": 1, "refresh_token_encrypted": 1,
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
            "access_token_encrypted": mongodb::bson::Binary {
                subtype: mongodb::bson::spec::BinarySubtype::Generic, bytes: vec![1, 2, 3],
            },
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
            connection_id: None,
            access_token_encrypted: Some(vec![1, 2, 3]),
            refresh_token_encrypted: None,
        }
    }

    #[test]
    fn oauth_app_metadata_requires_evidence_for_modern_keys() {
        let mut key = key();
        key.expires_at = Some(chrono::Utc::now() + chrono::Duration::days(365));
        assert_eq!(from_key(&key), None);
        key.user_oauth_client_id_encrypted = Some(vec![1]);
        assert_eq!(from_key(&key), None);
        key.oauth_app_observation = Some(OAuthAppSource::Byo.observation(key.credential_epoch));
        assert_eq!(from_key(&key), Some(OAuthAppSource::Byo));
        key.credential_epoch += 1;
        assert_eq!(from_key(&key), None);
        key.credential_source = Some("platform".into());
        assert_eq!(from_key(&key), Some(OAuthAppSource::Platform));
        key.credential_source = Some("unsupported".into());
        assert_eq!(from_key(&key), None);
    }

    #[test]
    fn oauth_app_metadata_observation_survives_expiry_but_not_reauthorization_or_conflict() {
        let mut key = key();
        let observation = OAuthAppSource::Platform.observation(key.credential_epoch);
        key.last_authorized_at = Some(observation.observed_at);
        key.oauth_app_observation = Some(observation.clone());
        key.status = "revoked".into();
        key.expires_at = Some(chrono::Utc::now() - chrono::Duration::days(1));
        assert_eq!(from_key(&key), Some(OAuthAppSource::Platform));
        key.credential_source = Some("byo".into());
        assert_eq!(from_key(&key), None);
        key.credential_source = None;
        key.access_token_encrypted = None;
        assert_eq!(from_key(&key), None);
        key.access_token_encrypted = Some(vec![1, 2, 3]);
        key.last_authorized_at = Some(observation.observed_at + chrono::Duration::seconds(1));
        assert_eq!(from_key(&key), None);
    }

    #[tokio::test]
    async fn oauth_app_metadata_batch_matches_actual_migrated_tokens() {
        let db = crate::test_utils::connect_test_database("oauth_app_metadata")
            .await
            .unwrap();
        let mut migrated = key();
        migrated.source = Some("migration_provider_token".into());
        migrated.source_id = Some("original".into());
        // A migrated modern key may carry an unrelated retained app.
        migrated.user_oauth_client_id_encrypted = Some(vec![99]);
        let mut changed = migrated.clone();
        changed.id = "reauthorized".into();
        changed.access_token_encrypted = Some(vec![8, 9]);
        let mut wrong_owner = migrated.clone();
        wrong_owner.id = "wrong-owner".into();
        wrong_owner.user_id = "different-owner".into();
        let mut legacy = key();
        legacy.id = "legacy-byo".into();
        legacy.connection_id = None;
        legacy.user_id = "org".into();
        let mut explicit = migrated.clone();
        explicit.id = "explicit".into();
        explicit.credential_source = Some("byo".into());
        db.collection::<mongodb::bson::Document>(COLLECTION_NAME)
            .insert_many([
                doc! {
                    "_id": "original", "user_id": "person", "provider_config_id": "x",
                    "access_token_encrypted": mongodb::bson::Binary {
                        subtype: mongodb::bson::spec::BinarySubtype::Generic, bytes: vec![1, 2, 3],
                    },
                },
                doc! {
                    "_id": "org-token", "user_id": "org", "provider_config_id": "x",
                    "credential_user_id": "org",
                    "access_token_encrypted": mongodb::bson::Binary {
                        subtype: mongodb::bson::spec::BinarySubtype::Generic, bytes: vec![1, 2, 3],
                    },
                },
            ])
            .await
            .unwrap();
        let sources = load(
            &db,
            &[&migrated, &changed, &wrong_owner, &legacy, &explicit],
        )
        .await
        .unwrap();
        assert_eq!(sources.len(), 3);
        assert_eq!(sources[&migrated.id], OAuthAppSource::Platform);
        assert_eq!(sources[&legacy.id], OAuthAppSource::Byo);
        assert_eq!(sources[&explicit.id], OAuthAppSource::Byo);
        assert!(!sources.contains_key(&changed.id));
        assert!(!sources.contains_key(&wrong_owner.id));
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
