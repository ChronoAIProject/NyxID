//! Native entry credentials bind the actual adapter signing producer and live
//! owner-bound connection. OAuth refresh ciphertext is deliberately not identity.
use super::{
    channel_managed::PlatformCredentialBacking,
    channel_platform::{CredentialResolution, PlatformAdapter, PlatformVerifySecrets},
    channel_verification_service::CredentialSnapshot,
};
use crate::{
    errors::{AppError, AppResult},
    models::{
        channel_bot::ChannelBot, platform_credential::PlatformCredential, user_api_key::UserApiKey,
    },
};
use bson::{Bson, Document, doc};
use mongodb::{ClientSession, Database};
use zeroize::Zeroizing;

struct Guard {
    collection: &'static str,
    filter: Document,
}

pub(crate) struct EntryCredentialSnapshot {
    bot: CredentialSnapshot,
    shared: Option<PlatformCredential>,
    connection_identity: Option<Document>,
    connection_token: Option<Zeroizing<Vec<u8>>>,
    token_expires_at: Option<chrono::DateTime<chrono::Utc>>,
    guards: Vec<Guard>,
}

fn changed() -> AppError {
    AppError::Conflict("Native entry credentials changed; verify the entry again".into())
}

fn field(row: &Document, name: &str) -> Bson {
    row.get(name).cloned().unwrap_or(Bson::Null)
}

impl EntryCredentialSnapshot {
    pub(crate) async fn load(
        db: &Database,
        adapter: &dyn PlatformAdapter,
        bot: &ChannelBot,
    ) -> AppResult<Self> {
        let base = CredentialSnapshot::load(db, adapter, bot).await?;
        let mut result = Self {
            bot: base,
            shared: None,
            connection_identity: None,
            connection_token: None,
            token_expires_at: None,
            guards: Vec::new(),
        };
        if bot.credential_source != "connection" {
            return Ok(result);
        }
        let CredentialResolution::OAuthConnection {
            provider_slug,
            required_scopes,
        } = adapter.credential_resolution()
        else {
            return Err(changed());
        };
        let required_scopes =
            if bot.platform == "x" && super::channel_adapters::x::public_events_enabled(bot) {
                super::channel_adapters::x::PUBLIC_SCOPES
            } else {
                required_scopes
            };
        let id = bot.connection_id.as_deref().ok_or_else(changed)?;
        let connection = super::service_history::collection::<UserApiKey>(
            db,
            crate::models::user_api_key::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":id,"user_id":&bot.user_id})
        .await?
        .ok_or_else(changed)?;
        let provider = db
            .collection::<Document>(crate::models::provider_config::COLLECTION_NAME)
            .find_one(doc! {"slug":provider_slug,"is_active":true})
            .await?
            .ok_or_else(changed)?;
        let provider_id = provider.get_str("_id").map_err(|_| changed())?;
        if connection.credential_type != "oauth2"
            || connection.status != "active"
            || connection.provider_config_id.as_deref() != Some(provider_id)
            || connection.credential_source.as_deref() == Some("byo")
            || connection.user_oauth_client_id_encrypted.is_some()
            || connection.credential_epoch <= 0
            || required_scopes.iter().any(|scope| {
                !connection
                    .token_scopes
                    .as_deref()
                    .unwrap_or_default()
                    .split_whitespace()
                    .any(|granted| granted == *scope)
            })
        {
            return Err(changed());
        }
        let mut identity = doc! {"_id":&connection.id,"user_id":&connection.user_id,"provider_config_id":provider_id,
        "credential_epoch":connection.credential_epoch,"credential_type":"oauth2","status":"active",
        "credential_source":connection.credential_source.clone(),"token_scopes":connection.token_scopes.clone(),
        "user_oauth_client_id_encrypted":Bson::Null};
        let mut connection_guard = identity.clone();
        // Refresh providers may reorder, repeat or space the same granted set.
        // Identity is semantic; the transaction still guards the exact snapshot.
        let scopes = connection
            .token_scopes
            .as_deref()
            .unwrap_or_default()
            .split_whitespace()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|scope| Bson::String(scope.to_owned()))
            .collect::<Vec<_>>();
        identity.insert("token_scopes", scopes);
        // An absent pre-epoch row means epoch1, exactly as UserApiKey serde does.
        if connection.credential_epoch == 1 {
            connection_guard.insert("credential_epoch", doc! {"$in":[1_i64,Bson::Null]});
        }

        result.connection_token = Some(Zeroizing::new(
            connection
                .access_token_encrypted
                .clone()
                .ok_or_else(changed)?,
        ));
        result.token_expires_at = connection.expires_at;
        result.connection_identity = Some(identity);
        result.guards.push(Guard {
            collection: crate::models::user_api_key::COLLECTION_NAME,
            filter: connection_guard,
        });
        result.guards.push(Guard {
            collection: crate::models::user::COLLECTION_NAME,
            filter: doc! {"_id":&bot.user_id,"is_active":true},
        });
        let mut provider_guard = doc! {"_id":provider_id,"slug":provider_slug,"is_active":true};
        for name in [
            "provider_type",
            "client_id_encrypted",
            "client_secret_encrypted",
            "updated_at",
        ] {
            provider_guard.insert(name, field(&provider, name));
        }
        if adapter.platform_webhook() {
            let descriptor = adapter.platform_credentials().ok_or_else(changed)?;
            if !matches!(descriptor.backing,PlatformCredentialBacking::ProviderOAuth{provider_slug:slug} if slug==provider_slug)
            {
                return Err(changed());
            }
            // Use the same composed producer as channel_managed::build_verify_secrets.
            // Its decrypted map contains provider OAuth signing keys plus auxiliary
            // legacy/app keys, rather than bot-local registration-field fallbacks.
            let shared = super::platform_credential_service::load(db, &descriptor)
                .await?
                .ok_or_else(changed)?;
            if shared.id != provider_id {
                return Err(changed());
            }
            for (name, path) in [
                ("client_id", "client_id_encrypted"),
                ("client_secret", "client_secret_encrypted"),
            ] {
                let expected = shared
                    .secrets
                    .get(name)
                    .cloned()
                    .map(Bson::Binary)
                    .unwrap_or(Bson::Null);
                if expected != field(&provider, path) {
                    return Err(changed());
                }
            }
            let auxiliary = db
                .collection::<Document>(crate::models::platform_credential::COLLECTION_NAME)
                .find_one(doc! {"provider":descriptor.provider})
                .await?;
            for secret in descriptor.fields.iter().filter(|field| {
                !matches!(field.name, "client_id" | "client_secret") && field.secret
            }) {
                let expected = shared
                    .secrets
                    .get(secret.name)
                    .cloned()
                    .map(Bson::Binary)
                    .unwrap_or(Bson::Null);
                let actual = auxiliary
                    .as_ref()
                    .and_then(|row| row.get_document("secrets").ok())
                    .map(|secrets| field(secrets, secret.name))
                    .unwrap_or(Bson::Null);
                if expected != actual {
                    return Err(changed());
                }
            }
            if let Some(auxiliary) = auxiliary {
                result.guards.push(Guard {collection:crate::models::platform_credential::COLLECTION_NAME,
                    filter:doc! {"_id":field(&auxiliary,"_id"),"provider":descriptor.provider,"secrets":field(&auxiliary,"secrets")}});
            }
            // All supported composite credential mutations also write the provider
            // row transactionally, including insertion/removal of auxiliary keys.
            // Its updated_at guard therefore fences even an initially absent map.
            result.shared = Some(shared);
        }
        result.guards.push(Guard {
            collection: crate::models::provider_config::COLLECTION_NAME,
            filter: provider_guard,
        });
        Ok(result)
    }

    pub(crate) fn fingerprint(&self) -> AppResult<String> {
        use sha2::{Digest, Sha256};
        let shared = self.shared.as_ref().map(|shared| &shared.secrets);
        let material = Zeroizing::new(
            serde_json::to_vec(&(
                "nyxid-native-entry-credentials:v2",
                self.bot.fingerprint()?,
                shared,
                &self.connection_identity,
            ))
            .map_err(|_| changed())?,
        );
        Ok(hex::encode(Sha256::digest(material.as_slice())))
    }

    /// Materialize exactly the encrypted credential captured before the final
    /// claim. No live same-ID connection read can substitute a successor epoch.
    pub(crate) async fn send_token(
        &self,
        keys: &crate::crypto::aes::EncryptionKeys,
    ) -> AppResult<Zeroizing<String>> {
        if self
            .token_expires_at
            .is_some_and(|expiry| expiry <= chrono::Utc::now())
        {
            return Err(changed());
        }
        if let Some(ciphertext) = &self.connection_token {
            let bytes = Zeroizing::new(keys.decrypt(ciphertext).await?);
            let token = std::str::from_utf8(&bytes).map_err(|_| changed())?;
            if token.is_empty() {
                return Err(changed());
            }
            return Ok(Zeroizing::new(token.into()));
        }
        self.bot.send_token(keys).await
    }

    pub(crate) async fn secrets(
        &self,
        keys: &crate::crypto::aes::EncryptionKeys,
        adapter: &dyn PlatformAdapter,
    ) -> AppResult<PlatformVerifySecrets> {
        if let Some(shared) = &self.shared {
            return super::platform_credential_service::decrypt_snapshot(keys, Some(shared)).await;
        }
        self.bot.secrets(keys, adapter).await
    }

    pub(crate) async fn fence_in_session(
        &self,
        db: &Database,
        session: &mut ClientSession,
    ) -> AppResult<bool> {
        if !self.fence_dependencies(db, session, true).await? {
            return Ok(false);
        }
        self.bot.fence_in_session(db, session).await
    }

    /// Administrative refresh admission checks persistent account authority;
    /// expired token material is never authorized for a provider send here.
    pub(super) async fn fence_refresh_dependencies_in_session(
        &self,
        db: &Database,
        session: &mut ClientSession,
    ) -> AppResult<bool> {
        self.fence_dependencies(db, session, false).await
    }

    async fn fence_dependencies(
        &self,
        db: &Database,
        session: &mut ClientSession,
        require_live: bool,
    ) -> AppResult<bool> {
        for guard in &self.guards {
            let mut filter = guard.filter.clone();
            if require_live && guard.collection == crate::models::user_api_key::COLLECTION_NAME {
                filter.insert(
                    "$or",
                    vec![
                        doc! {"expires_at":Bson::Null},
                        doc! {"expires_at":{"$gt":bson::DateTime::now()}},
                    ],
                );
            }
            if db
                .collection::<Document>(guard.collection)
                .update_one(
                    filter,
                    doc! {"$inc":{"platform_entry_effect_revision":1_i64}},
                )
                .session(&mut *session)
                .await?
                .matched_count
                != 1
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
