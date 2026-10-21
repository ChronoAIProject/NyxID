//! Additional managers use distinct provider rows, invisible to older replicas.
use super::{channel_platform::PlatformVerifySecrets, telegram_new_service::TelegramNewService};
use crate::{
    errors::{AppError, AppResult},
    models::platform_credential::{COLLECTION_NAME, PlatformCredential},
};
use mongodb::{
    Database,
    bson::{self, doc},
};
use zeroize::Zeroizing;

pub fn filter(manager: &str) -> bson::Document {
    doc! {"provider":{"$in":["telegram-new".to_owned(),format!("telegram-new/{manager}")]},"fields.manager_bot_id":manager}
}
pub async fn load(db: &Database, manager: Option<&str>) -> AppResult<Option<PlatformCredential>> {
    let filter = manager
        .map(filter)
        .unwrap_or_else(|| doc! {"provider":"telegram-new"});
    Ok(db
        .collection::<PlatformCredential>(COLLECTION_NAME)
        .find_one(filter)
        .await?)
}
pub async fn values(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    manager: Option<&str>,
) -> AppResult<PlatformVerifySecrets> {
    super::platform_credential_service::decrypt_platform_snapshot(
        keys,
        load(db, manager).await?.as_ref(),
    )
    .await
}
impl<'a> TelegramNewService<'a> {
    pub async fn for_manager(&self, manager: i64) -> AppResult<Self> {
        let row = load(self.db, Some(&manager.to_string()))
            .await?
            .ok_or_else(|| AppError::NotFound("Telegram manager unavailable".into()))?;
        Ok(Self {
            db: self.db,
            keys: self.keys,
            config: self.config,
            api: super::telegram_new_api::TelegramApi {
                http: self.api.http,
                base_url: self.api.base_url,
            },
            manager_id: (row.provider != "telegram-new").then_some(manager),
        })
    }
    pub(crate) async fn manager_values(&self) -> AppResult<PlatformVerifySecrets> {
        let row = self
            .db
            .collection::<PlatformCredential>(COLLECTION_NAME)
            .find_one(doc! {"provider":self.manager_provider()})
            .await?;
        super::platform_credential_service::decrypt_platform_snapshot(self.keys, row.as_ref()).await
    }
    pub(crate) fn manager_provider(&self) -> String {
        self.manager_id
            .map_or_else(|| "telegram-new".into(), |id| format!("telegram-new/{id}"))
    }
    pub(crate) async fn save_manager_fields(
        &self,
        actor: &str,
        fields: &std::collections::BTreeMap<String, Option<Zeroizing<String>>>,
    ) -> AppResult<()> {
        let Some(id) = self.manager_id else {
            return super::platform_credential_service::update(
                self.db,
                self.keys,
                &super::channel_adapters::telegram_new::credential_descriptor(),
                actor,
                fields,
                false,
            )
            .await
            .map(|_| ());
        };
        if id <= 0 {
            return Err(AppError::ValidationError("Invalid manager identity".into()));
        }
        if load(self.db, Some(&id.to_string()))
            .await?
            .is_some_and(|r| r.provider == "telegram-new")
        {
            return Err(AppError::Conflict(
                "This manager is the existing default. Configure it in Platform Credentials."
                    .into(),
            ));
        }
        let source = self
            .db
            .collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {"provider":self.manager_provider()})
            .await?;
        let previous = source
            .clone()
            .map(bson::from_document::<PlatformCredential>)
            .transpose()
            .map_err(|_| AppError::Internal("Invalid platform credential storage".into()))?;
        let mut row = previous.unwrap_or_else(|| PlatformCredential {
            secret_context_versions: Default::default(),
            id: uuid::Uuid::new_v4().to_string(),
            channel_observation_version: 0,
            provider: self.manager_provider(),
            fields: Default::default(),
            secrets: Default::default(),
            updated_by: actor.into(),
            updated_at: chrono::Utc::now(),
        });
        if let Some(token) = fields
            .get(super::channel_adapters::telegram_new::MANAGER_TOKEN)
            .and_then(Option::as_ref)
        {
            row.secrets.insert(
                super::channel_adapters::telegram_new::MANAGER_TOKEN.into(),
                bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: super::platform_credential_service::encrypt_platform_field(
                        self.keys,
                        &row.id,
                        super::channel_adapters::telegram_new::MANAGER_TOKEN,
                        token.trim().as_bytes(),
                    )
                    .await?,
                },
            );
            super::credential_record_service::mark_version(
                &mut row.secret_context_versions,
                self.keys,
                super::channel_adapters::telegram_new::MANAGER_TOKEN,
            );
        }
        if !row
            .secrets
            .contains_key(super::channel_adapters::telegram_new::MANAGER_TOKEN)
        {
            return Err(AppError::ValidationError("Manager token required".into()));
        }
        if !row
            .secrets
            .contains_key(super::platform_credential_service::VERIFY_TOKEN_FIELD)
        {
            let secret = Zeroizing::new(hex::encode(rand::random::<[u8; 32]>()));
            row.secrets.insert(
                super::platform_credential_service::VERIFY_TOKEN_FIELD.into(),
                bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: super::platform_credential_service::encrypt_platform_field(
                        self.keys,
                        &row.id,
                        super::platform_credential_service::VERIFY_TOKEN_FIELD,
                        secret.as_bytes(),
                    )
                    .await?,
                },
            );
            super::credential_record_service::mark_version(
                &mut row.secret_context_versions,
                self.keys,
                super::platform_credential_service::VERIFY_TOKEN_FIELD,
            );
        }
        row.fields.insert("manager_bot_id".into(), id.to_string());
        row.updated_by = actor.into();
        row.updated_at = chrono::Utc::now();
        save_scoped_snapshot(self.db, actor, source.as_ref(), &row).await
    }
}

async fn save_scoped_snapshot(
    db: &Database,
    actor: &str,
    source: Option<&bson::Document>,
    row: &PlatformCredential,
) -> AppResult<()> {
    let mut candidate = source.cloned().unwrap_or_default();
    candidate.extend(
        bson::to_document(row)
            .map_err(|_| AppError::Internal("Invalid platform credential storage".into()))?,
    );
    let filter = source
        .map(super::record_storage::exact_document_predicate)
        .unwrap_or_else(|| doc! {"_id":&row.id,"provider":&row.provider});
    let db = super::transaction_service::durable_database(db);
    let context = (&db, actor, &filter, &candidate, source.is_none());
    super::transaction_service::run_db_only(&db, &context, |tx, context| {
        let &(db, actor, filter, candidate, upsert) = context;
        Box::pin(async move {
            super::platform_bot_config_service::fence_admin(db, actor, tx).await?;
            let changed = db
                .collection::<bson::Document>(COLLECTION_NAME)
                .replace_one(filter.clone(), candidate.clone())
                .upsert(upsert)
                .session(tx)
                .await?;
            if changed.matched_count == 0 && changed.upserted_id.is_none() {
                return Err(AppError::Conflict(
                    "Platform credentials changed; reload and retry".into(),
                ));
            }
            Ok(())
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::{
        Mock, ResponseTemplate,
        matchers::{method, path},
    };

    #[tokio::test]
    async fn manager_capacity_includes_a_new_default_before_any_provider_effect() {
        let (state, actor, server) = super::super::telegram_new_tests::fixture().await;
        super::super::role_service::seed_system_roles(&state.db)
            .await
            .unwrap();
        let role = super::super::role_service::get_platform_role_ids(&state.db)
            .await
            .unwrap()
            .admin;
        state
            .db
            .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .update_one(doc! {"_id":&actor}, doc! {"$set":{"role_ids":[role]}})
            .await
            .unwrap();
        let rows = state.db.collection::<PlatformCredential>(COLLECTION_NAME);
        let template = rows
            .find_one_and_delete(doc! {"provider":"telegram-new"})
            .await
            .unwrap()
            .unwrap();
        let managers: Vec<_> = (1000..1100)
            .map(|id| {
                let mut row = template.clone();
                row.id = uuid::Uuid::new_v4().to_string();
                row.provider = format!("telegram-new/{id}");
                row.fields.insert("manager_bot_id".into(), id.to_string());
                row
            })
            .collect();
        rows.insert_many(managers).await.unwrap();
        let base = server.uri();
        let fields = std::collections::BTreeMap::from([(
            "manager_bot_token".into(),
            Some(Zeroizing::new("100:manager-test-secret".into())),
        )]);
        for manager_id in [None, Some(100)] {
            let mut service = super::super::telegram_new_tests::service(&state, &base);
            service.manager_id = manager_id;
            let error = service
                .configure_manager(&actor, &fields, false)
                .await
                .unwrap_err();
            assert!(matches!(error, AppError::Conflict(ref message)
                if message == "At most 100 Telegram managers are supported."));
            assert_eq!(rows.count_documents(doc! {}).await.unwrap(), 100);
        }
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "Capacity must be checked before getMe or webhook changes"
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn managers_keep_separate_tokens_secrets_observations_and_cleanup() {
        let (state, actor, server) = super::super::telegram_new_tests::fixture().await;
        super::super::role_service::seed_system_roles(&state.db)
            .await
            .unwrap();
        let role = super::super::role_service::get_platform_role_ids(&state.db)
            .await
            .unwrap()
            .admin;
        state
            .db
            .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .update_one(doc! {"_id":&actor}, doc! {"$set":{"role_ids":[role]}})
            .await
            .unwrap();
        let base = server.uri();
        let default = super::super::telegram_new_tests::service(&state, &base);
        let default_before = default.manager_values().await.unwrap();
        let mut additional = super::super::telegram_new_tests::service(&state, &base);
        additional.manager_id = Some(200);
        let token = "200:fixture-manager-token";
        for (operation, result) in [
            (
                "getMe",
                json!({"id":200,"is_bot":true,"username":"SecondManagerBot","can_manage_bots":true}),
            ),
            (
                "getWebhookInfo",
                json!({"url":additional.manager_callback(),"allowed_updates":super::super::telegram_new_admin::MANAGER_ALLOWED_UPDATES}),
            ),
            ("setWebhook", json!(true)),
            ("deleteWebhook", json!(true)),
        ] {
            Mock::given(method("POST"))
                .and(path(format!("/bot{token}/{operation}")))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":result})),
                )
                .mount(&server)
                .await;
        }
        let fields = std::collections::BTreeMap::from([(
            "manager_bot_token".into(),
            Some(Zeroizing::new(token.into())),
        )]);
        additional
            .configure_manager(&actor, &fields, false)
            .await
            .unwrap();
        let rows = state.db.collection::<bson::Document>(COLLECTION_NAME);
        let source = rows
            .find_one(doc! {"provider":"telegram-new/200"})
            .await
            .unwrap()
            .unwrap();
        let stale: PlatformCredential = bson::from_document(source.clone()).unwrap();
        // A concurrent key rewrap changes ciphertext while preserving the token.
        let token_field = super::super::channel_adapters::telegram_new::MANAGER_TOKEN;
        let rewrapped = super::super::platform_credential_service::encrypt_platform_field(
            &state.encryption_keys,
            &stale.id,
            token_field,
            token.as_bytes(),
        )
        .await
        .unwrap();
        rows.update_one(doc! {"_id":&stale.id}, doc! {"$set":{
            "secrets.manager_bot_token":bson::Binary {subtype:bson::spec::BinarySubtype::Generic, bytes:rewrapped},
            "future_storage_marker":"preserve"
        }}).await.unwrap();
        assert!(matches!(
            save_scoped_snapshot(&state.db, &actor, Some(&source), &stale).await,
            Err(AppError::Conflict(_))
        ));
        let concurrent = rows
            .find_one(doc! {"_id":&stale.id})
            .await
            .unwrap()
            .unwrap();
        assert_ne!(
            concurrent.get_document("secrets").unwrap(),
            source.get_document("secrets").unwrap()
        );
        additional
            .configure_manager(&actor, &fields, false)
            .await
            .unwrap();
        let refreshed = rows
            .find_one(doc! {"_id":&stale.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            refreshed.get_str("future_storage_marker").unwrap(),
            "preserve"
        );
        let saved = additional.manager_values().await.unwrap();
        assert_eq!(saved.get("manager_bot_id"), Some("200"));
        let mut strict_keys = crate::crypto::aes::EncryptionKeys::from_config(&state.config);
        strict_keys.set_allow_legacy_record_reads(false);
        let mut snapshot = load(&state.db, Some("200")).await.unwrap().unwrap();
        let strict = super::super::platform_credential_service::decrypt_platform_snapshot(
            &strict_keys,
            Some(&snapshot),
        )
        .await
        .unwrap();
        assert_eq!(strict.get("manager_bot_token"), Some(token));
        snapshot.id = uuid::Uuid::new_v4().to_string();
        assert!(
            super::super::platform_credential_service::decrypt_platform_snapshot(
                &strict_keys,
                Some(&snapshot),
            )
            .await
            .is_err()
        );
        let verify = super::super::platform_credential_service::VERIFY_TOKEN_FIELD;
        assert_ne!(saved.get(verify), default_before.get(verify));
        assert_eq!(
            default.manager_values().await.unwrap().get(verify),
            default_before.get(verify)
        );
        let selected = default.for_manager(200).await.unwrap();
        assert_eq!(selected.manager_id, Some(200));
        assert_eq!(selected.manager().await.unwrap().0, 200);
        assert_eq!(default.for_manager(100).await.unwrap().manager_id, None);
        let wrong = super::super::telegram_new_tests::headers(&state).await;
        assert!(selected.webhook(&wrong, br#"{}"#).await.is_err());
        let own = axum::http::HeaderMap::from_iter([(
            "x-telegram-bot-api-secret-token".parse().unwrap(),
            saved.get(verify).unwrap().parse().unwrap(),
        )]);
        assert!(selected.webhook(&own, br#"{}"#).await.unwrap().is_none());
        // A failed webhook save keeps its identity and secret for repair.
        state
            .db
            .collection::<bson::Document>(COLLECTION_NAME)
            .update_one(
                doc! {"provider":"telegram-new/200"},
                doc! {"$set":{"fields.webhook_ready":"false"}},
            )
            .await
            .unwrap();
        additional
            .configure_manager(&actor, &Default::default(), false)
            .await
            .unwrap();
        assert_eq!(
            additional.manager_values().await.unwrap().get(verify),
            saved.get(verify)
        );
        assert_eq!(
            additional
                .manager_values()
                .await
                .unwrap()
                .get("observation_id"),
            saved.get("observation_id")
        );
        // Pending work for the default manager cannot block clearing the other.
        state.db.collection::<bson::Document>(crate::models::telegram_bot_request::COLLECTION_NAME)
            .insert_one(doc!{"_id":uuid::Uuid::new_v4().to_string(),"manager_bot_id":100_i64,"active":true,"status":"waiting_bot","expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::hours(1))}).await.unwrap();
        additional.clear_manager_as(&actor).await.unwrap();
        assert!(load(&state.db, Some("200")).await.unwrap().is_none());
        assert!(default.manager().await.is_ok());
        state.db.drop().await.unwrap();
    }
}
