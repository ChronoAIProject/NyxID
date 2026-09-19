//! Credential checks observe the exact encrypted snapshot used by the adapter.

use bson::{Document, doc};
use chrono::Utc;
use sha2::{Digest, Sha256};

use super::channel_platform::{BotCredentials, PlatformAdapter, PlatformVerifySecrets};
use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::{
    BotVerification, COLLECTION_NAME, ChannelBot, VerificationStatus,
};
use crate::models::platform_credential::PlatformCredential;

pub const VERIFICATION_TIMEOUT_SECS: u64 = 30;

fn changed() -> AppError {
    AppError::Conflict("Bot credentials or verification changed. Run verification again.".into())
}

/// Compare encrypted material directly in MongoDB; never log this filter.
fn credential_filter(bot: &ChannelBot) -> AppResult<Document> {
    let stored = bson::to_document(bot)
        .map_err(|_| AppError::Internal("Unable to encode bot credentials".into()))?;
    let mut filter = doc! {};
    for field in [
        "_id",
        "user_id",
        "platform",
        "platform_bot_id",
        "app_id",
        "connection_id",
        "bot_token_encrypted",
        "app_secret_encrypted",
        "webhook_secret_hash",
        "lark_verification_token_encrypted",
        "lark_encrypt_key_encrypted",
        "public_key",
        "is_active",
        "status",
    ] {
        filter.insert(
            field,
            stored.get(field).cloned().unwrap_or(bson::Bson::Null),
        );
    }
    filter.insert(
        "credential_source",
        if bot.credential_source == "user" {
            bson::Bson::Document(doc! { "$in": ["user", bson::Bson::Null] })
        } else {
            bson::Bson::String(bot.credential_source.clone())
        },
    );
    Ok(filter)
}

pub(crate) struct CredentialSnapshot {
    bot: ChannelBot,
    platform: Option<PlatformCredential>,
}

impl CredentialSnapshot {
    pub(crate) fn local(bot: &ChannelBot) -> Self {
        Self {
            bot: bot.clone(),
            platform: None,
        }
    }
    pub(crate) async fn load(
        db: &mongodb::Database,
        adapter: &dyn PlatformAdapter,
        bot: &ChannelBot,
    ) -> AppResult<Self> {
        let platform = if bot.credential_source == "platform" {
            let descriptor = adapter
                .platform_credentials()
                .ok_or_else(super::channel_managed::unavailable)?;
            super::platform_credential_service::load(db, &descriptor).await?
        } else {
            None
        };
        Ok(Self {
            bot: bot.clone(),
            platform,
        })
    }

    fn fingerprint(&self) -> AppResult<String> {
        let mut fields = credential_filter(&self.bot)?;
        // A matching webhook can activate a bot while verification is in flight.
        fields.remove("status");
        fields.remove("is_active");
        if self.bot.credential_source == "platform" {
            fields.insert(
                "platform_credentials",
                match &self.platform {
                    Some(platform) => bson::to_bson(&platform.secrets).map_err(|_| {
                        AppError::Internal("Unable to encode platform credentials".into())
                    })?,
                    None => bson::Bson::Null,
                },
            );
        }
        let bytes = bson::to_vec(&fields)
            .map_err(|_| AppError::Internal("Unable to encode credential check".into()))?;
        Ok(hex::encode(Sha256::digest(bytes)))
    }

    pub(crate) async fn secrets(
        &self,
        keys: &crate::crypto::aes::EncryptionKeys,
        adapter: &dyn PlatformAdapter,
    ) -> AppResult<PlatformVerifySecrets> {
        let mut secrets = adapter.build_verify_secrets(keys, &self.bot).await?;
        if self.bot.credential_source == "platform" {
            let platform =
                super::platform_credential_service::decrypt_snapshot(keys, self.platform.as_ref())
                    .await?;
            for field in adapter.registration().fields {
                if secrets.get(field.name).is_none()
                    && let Some(fallback) = field.platform_fallback
                    && let Some(value) = platform.get(fallback)
                {
                    secrets.insert(field.name, value.to_string());
                }
            }
        }
        Ok(secrets)
    }

    async fn update(
        &self,
        db: &mongodb::Database,
        filter: Document,
        update: impl Into<mongodb::options::UpdateModifications>,
    ) -> AppResult<bool> {
        let update = update.into();
        let Some(platform) = &self.platform else {
            return Ok(db
                .collection::<ChannelBot>(COLLECTION_NAME)
                .update_one(filter, update)
                .await?
                .matched_count
                == 1);
        };
        let guard = doc! { "_id": &platform.id, "secrets": bson::to_bson(&platform.secrets).map_err(|_| AppError::Internal("Unable to encode platform credentials".into()))? };
        let mut session = db.client().start_session().await?;
        // Writing the shared credential row makes rotation conflict with this transaction.
        // A snapshot read alone would permit an old signature to activate after rotation.
        let matched = session
            .start_transaction()
            .and_run(
                (db.clone(), guard, filter, update),
                |session, (db, guard, filter, update)| {
                    Box::pin(async move {
                        let locked = db
                            .collection::<Document>(
                                crate::models::platform_credential::COLLECTION_NAME,
                            )
                            .update_one(
                                guard.clone(),
                                doc! { "$inc": { "channel_observation_version": 1_i64 } },
                            )
                            .session(&mut *session)
                            .await?;
                        if locked.matched_count != 1 {
                            return Ok(false);
                        }
                        let result = db
                            .collection::<ChannelBot>(COLLECTION_NAME)
                            .update_one(filter.clone(), update.clone())
                            .session(&mut *session)
                            .await?;
                        Ok(result.matched_count == 1)
                    })
                },
            )
            .await?;
        Ok(matched)
    }

    pub(crate) async fn activate(&self, db: &mongodb::Database) -> AppResult<bool> {
        if !self.bot.is_active || self.bot.status != "pending_webhook" {
            return Ok(false);
        }
        let mut filter = credential_filter(&self.bot)?;
        filter.insert("status", doc! { "$in": ["pending_webhook", "active"] });
        self.update(db, filter, vec![doc! { "$set": {
            "updated_at": { "$cond": [{ "$eq": ["$status", "pending_webhook"] }, bson::DateTime::now(), "$updated_at"] },
            "status": "active", "webhook_registered": true,
        } }]).await
    }
}

pub async fn current(
    db: &mongodb::Database,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
) -> AppResult<Option<BotVerification>> {
    let Some(check) = &bot.last_verification else {
        return Ok(None);
    };
    if !bot.is_active
        || check.credential_fingerprint
            != CredentialSnapshot::load(db, adapter, bot)
                .await?
                .fingerprint()?
    {
        return Ok(None);
    }
    let mut observed = check.clone();
    if matches!(observed.status, VerificationStatus::Pending)
        && Utc::now()
            >= observed.started_at + chrono::Duration::seconds(VERIFICATION_TIMEOUT_SECS as i64)
    {
        observed.status = VerificationStatus::Incomplete;
        observed.message =
            Some("The last credential check did not complete. Run verification again.".into());
    }
    Ok(Some(observed))
}

async fn begin(
    db: &mongodb::Database,
    snapshot: &CredentialSnapshot,
) -> AppResult<BotVerification> {
    if !snapshot.bot.is_active {
        return Err(AppError::ChannelBotInactive("Bot is inactive".into()));
    }
    let check = BotVerification {
        attempt_id: uuid::Uuid::new_v4().to_string(),
        credential_fingerprint: snapshot.fingerprint()?,
        status: VerificationStatus::Pending,
        message: None,
        started_at: bson::DateTime::now().to_chrono(),
        completed_at: None,
    };
    if !snapshot.update(db, credential_filter(&snapshot.bot)?, doc! { "$set": { "last_verification": bson::to_bson(&check).map_err(|_| AppError::Internal("Unable to encode credential check".into()))? } }).await? {
        return Err(changed());
    }
    Ok(check)
}

async fn finish(
    db: &mongodb::Database,
    snapshot: &CredentialSnapshot,
    check: &BotVerification,
    result: &AppResult<()>,
) -> AppResult<BotVerification> {
    let mut finished = check.clone();
    finished.completed_at = Some(Utc::now());
    finished.status = if result.is_ok() {
        VerificationStatus::Verified
    } else {
        VerificationStatus::Failed
    };
    finished.message = result.as_ref().err().map(|error| match error {
        // WhatsApp's platform errors contain locally authored prose only.
        AppError::ChannelPlatformError(message) => message.clone(),
        _ => "Credential verification could not complete. Check the configured credentials and retry.".into(),
    });
    let mut filter = credential_filter(&snapshot.bot)?;
    if snapshot.bot.status == "pending_webhook" {
        filter.insert("status", doc! { "$in": ["pending_webhook", "active"] });
    }
    filter.insert("last_verification.attempt_id", &check.attempt_id);
    if !snapshot.update(db, filter, doc! { "$set": { "last_verification": bson::to_bson(&finished).map_err(|_| AppError::Internal("Unable to encode credential check".into()))? } }).await? {
        return Err(changed());
    }
    Ok(finished)
}

pub async fn verify(
    db: &mongodb::Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    http: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
) -> AppResult<BotVerification> {
    let snapshot = CredentialSnapshot::load(db, adapter, bot).await?;
    let check = begin(db, &snapshot).await?;
    let deadline = check.started_at + chrono::Duration::seconds(VERIFICATION_TIMEOUT_SECS as i64);
    let remaining = (deadline - Utc::now()).to_std().unwrap_or_default();
    let result = tokio::time::timeout(remaining, async {
        let token =
            super::channel_credentials::resolve_bot_token(db, keys, adapter, &snapshot.bot).await?;
        let secrets = if bot.credential_source == "platform" {
            Some(snapshot.secrets(keys, adapter).await?)
        } else {
            None
        };
        adapter.validate_stored_verification(&snapshot.bot)?;
        adapter
            .verify_bot_token(
                http,
                &BotCredentials {
                    token: &token,
                    platform_bot_id: Some(&snapshot.bot.platform_bot_id),
                    platform_secrets: secrets.as_ref(),
                },
            )
            .await?;
        Ok(())
    })
    .await
    .unwrap_or_else(|_| {
        Err(AppError::ChannelPlatformError(
            "Credential verification timed out. Retry the check.".into(),
        ))
    });
    let finished = finish(db, &snapshot, &check, &result).await?;
    result?;
    Ok(finished)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::channel_adapters::whatsapp::WhatsAppAdapter;

    async fn fixture(managed: bool) -> (mongodb::Database, crate::AppState, ChannelBot) {
        let db = crate::test_utils::connect_transaction_test_database("wa_verification").await;
        let state = crate::test_utils::test_app_state(db.clone());
        let encrypted = state
            .encryption_keys
            .encrypt(b"test-bot-token")
            .await
            .unwrap();
        let secret = state
            .encryption_keys
            .encrypt(b"test-app-secret")
            .await
            .unwrap();
        let bot: ChannelBot = bson::from_document(doc! {
            "_id": uuid::Uuid::new_v4().to_string(), "user_id": uuid::Uuid::new_v4().to_string(), "platform": "whatsapp", "label": "test",
            "platform_bot_id": "123", "platform_bot_username": "+123", "app_id": "456",
            "credential_source": if managed { "platform" } else { "user" },
            "bot_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: encrypted },
            "app_secret_encrypted": if managed { bson::Bson::Null } else { bson::Bson::Binary(bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: secret.clone() }) },
            "webhook_secret_hash": "unchanged-verify-token-hash", "webhook_registered": false, "status": "pending_webhook", "is_active": true,
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now()
        }).unwrap();
        db.collection::<ChannelBot>(COLLECTION_NAME)
            .insert_one(&bot)
            .await
            .unwrap();
        if managed {
            db.collection::<Document>(crate::models::platform_credential::COLLECTION_NAME).insert_one(doc! {
                "_id": uuid::Uuid::new_v4().to_string(), "provider":"meta", "fields":{}, "secrets": {"app_secret": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: secret }},
                "updated_by":"test", "updated_at":bson::DateTime::now()
            }).await.unwrap();
        }
        (db, state, bot)
    }

    async fn read(db: &mongodb::Database, bot: &ChannelBot) -> ChannelBot {
        super::super::channel_bot_service::get_bot(db, &bot.id)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn verification_survives_matching_activation_and_supersedes_previous_attempts() {
        for managed in [false, true] {
            let (db, _, bot) = fixture(managed).await;
            let snapshot = CredentialSnapshot::load(&db, &WhatsAppAdapter, &bot)
                .await
                .unwrap();
            let first = begin(&db, &snapshot).await.unwrap();
            let latest = begin(&db, &snapshot).await.unwrap();
            assert!(snapshot.activate(&db).await.unwrap());
            assert!(matches!(
                finish(&db, &snapshot, &first, &Ok(())).await,
                Err(AppError::Conflict(_))
            ));
            finish(&db, &snapshot, &latest, &Ok(())).await.unwrap();
            let active = read(&db, &bot).await;
            assert_eq!(active.status, "active");
            assert_eq!(active.webhook_secret_hash, bot.webhook_secret_hash);
            assert!(matches!(
                current(&db, &WhatsAppAdapter, &active)
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                VerificationStatus::Verified
            ));
            let next = CredentialSnapshot::load(&db, &WhatsAppAdapter, &active)
                .await
                .unwrap();
            let check = begin(&db, &next).await.unwrap();
            finish(
                &db,
                &next,
                &check,
                &Err(AppError::ChannelPlatformError(
                    "Access token is invalid or expired".into(),
                )),
            )
            .await
            .unwrap();
            let failed = read(&db, &bot).await;
            assert_eq!(failed.status, "active");
            assert!(matches!(
                failed.last_verification.unwrap().status,
                VerificationStatus::Failed
            ));
        }
    }

    #[tokio::test]
    async fn stale_pending_is_observed_as_incomplete_without_database_mutation() {
        let (db, _, bot) = fixture(false).await;
        let snapshot = CredentialSnapshot::load(&db, &WhatsAppAdapter, &bot)
            .await
            .unwrap();
        begin(&db, &snapshot).await.unwrap();
        db.collection::<ChannelBot>(COLLECTION_NAME).update_one(doc! {"_id":&bot.id}, doc! {"$set":{"last_verification.started_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(31))}}).await.unwrap();
        let pending = read(&db, &bot).await;
        let observed = current(&db, &WhatsAppAdapter, &pending)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(observed.status, VerificationStatus::Incomplete));
        assert!(matches!(
            read(&db, &bot).await.last_verification.unwrap().status,
            VerificationStatus::Pending
        ));
    }

    #[tokio::test]
    async fn bot_credential_rotation_and_disable_fence_activation_and_verification() {
        for field in [
            "bot_token_encrypted",
            "app_secret_encrypted",
            "is_active",
            "user_id",
        ] {
            let (db, state, bot) = fixture(false).await;
            let snapshot = CredentialSnapshot::load(&db, &WhatsAppAdapter, &bot)
                .await
                .unwrap();
            let check = begin(&db, &snapshot).await.unwrap();
            let value = match field {
                "is_active" => bson::Bson::Boolean(false),
                "user_id" => bson::Bson::String(uuid::Uuid::new_v4().to_string()),
                _ => bson::Bson::Binary(bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: state.encryption_keys.encrypt(b"replacement").await.unwrap(),
                }),
            };
            db.collection::<ChannelBot>(COLLECTION_NAME)
                .update_one(doc! {"_id":&bot.id}, doc! {"$set":{field:value}})
                .await
                .unwrap();
            assert!(!snapshot.activate(&db).await.unwrap());
            assert!(matches!(
                finish(&db, &snapshot, &check, &Ok(())).await,
                Err(AppError::Conflict(_))
            ));
            assert!(
                current(&db, &WhatsAppAdapter, &read(&db, &bot).await)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[tokio::test]
    async fn shared_secret_snapshot_is_used_and_rotation_fences_both_observations() {
        let (db, state, bot) = fixture(true).await;
        let snapshot = CredentialSnapshot::load(&db, &WhatsAppAdapter, &bot)
            .await
            .unwrap();
        let check = begin(&db, &snapshot).await.unwrap();
        let secret = state
            .encryption_keys
            .encrypt(b"replacement-shared-secret")
            .await
            .unwrap();
        db.collection::<Document>(crate::models::platform_credential::COLLECTION_NAME).update_one(doc! {"provider":"meta"},doc! {"$set":{"secrets.app_secret":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:secret}}}).await.unwrap();
        let actual = snapshot
            .secrets(&state.encryption_keys, &WhatsAppAdapter)
            .await
            .unwrap();
        assert!(actual.get("app_secret") == Some("test-app-secret"));
        assert!(!snapshot.activate(&db).await.unwrap());
        assert!(matches!(
            finish(&db, &snapshot, &check, &Ok(())).await,
            Err(AppError::Conflict(_))
        ));
        assert!(
            current(&db, &WhatsAppAdapter, &read(&db, &bot).await)
                .await
                .unwrap()
                .is_none()
        );
        let fresh = CredentialSnapshot::load(&db, &WhatsAppAdapter, &bot)
            .await
            .unwrap();
        assert!(
            fresh
                .secrets(&state.encryption_keys, &WhatsAppAdapter)
                .await
                .unwrap()
                .get("app_secret")
                == Some("replacement-shared-secret")
        );
        assert!(fresh.activate(&db).await.unwrap());
    }

    #[tokio::test]
    async fn concurrent_matching_activations_remain_compatible() {
        for managed in [false, true] {
            let (db, _, bot) = fixture(managed).await;
            let first = CredentialSnapshot::load(&db, &WhatsAppAdapter, &bot)
                .await
                .unwrap();
            let second = CredentialSnapshot::load(&db, &WhatsAppAdapter, &bot)
                .await
                .unwrap();
            let (a, b) = tokio::join!(first.activate(&db), second.activate(&db));
            assert!(a.unwrap());
            assert!(b.unwrap());
            let active = read(&db, &bot).await;
            assert!(first.activate(&db).await.unwrap());
            assert_eq!(read(&db, &bot).await.updated_at, active.updated_at);
            assert_eq!(active.status, "active");
        }
    }

    #[tokio::test]
    async fn credential_patch_invalidates_check_and_preserves_verify_token() {
        let (db, state, bot) = fixture(false).await;
        let snapshot = CredentialSnapshot::load(&db, &WhatsAppAdapter, &bot)
            .await
            .unwrap();
        let check = begin(&db, &snapshot).await.unwrap();
        let updated = super::super::channel_bot_service::update_bot(
            &db,
            &state.encryption_keys,
            &state.http_client,
            &WhatsAppAdapter,
            &bot.id,
            &bot.user_id,
            super::super::channel_bot_service::UpdateBotParams {
                bot_token: None,
                label: None,
                verification_token: None,
                encrypt_key: super::super::channel_bot_service::SecretPatch::Unchanged,
                app_id: None,
                app_secret: Some("replacement"),
            },
        )
        .await
        .unwrap();
        assert!(updated.last_verification.is_none());
        assert_eq!(updated.webhook_secret_hash, bot.webhook_secret_hash);
        assert!(finish(&db, &snapshot, &check, &Ok(())).await.is_err());
    }
}
