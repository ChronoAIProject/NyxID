//! Serialize the temporary channel-signing migration guard with bot registration.
//! Both operations write the singleton registration row before admitting a new binding.
use super::channel_managed::PlatformCredentialBacking;
use crate::errors::{AppError, AppResult};
use crate::models::{channel_bot, platform_credential, platform_settings, provider_config};
use bson::{Bson, Document, doc};
use mongodb::{ClientSession, Database};
use std::sync::Arc;

pub(super) async fn fence_registration(
    db: &Database,
    session: &mut ClientSession,
) -> AppResult<()> {
    db.collection::<Document>(platform_settings::COLLECTION_NAME)
        .update_one(
            doc! {"_id": platform_settings::PLATFORM_SETTINGS_ID},
            doc! {"$inc": {"channel_bot_registration_revision": 1_i64}},
        )
        .upsert(true)
        .session(session)
        .await?;
    Ok(())
}

pub(super) async fn guard(
    db: &Database,
    session: &mut ClientSession,
    collection: &str,
    source: &Document,
) -> AppResult<()> {
    if collection != provider_config::COLLECTION_NAME
        && collection != platform_credential::COLLECTION_NAME
    {
        return Ok(());
    }
    let cache = Arc::new(super::provider_token_exchange_service::TokenExchangeCache::new());
    let mut consumers = Vec::new();
    for adapter in super::channel_adapters::registered_adapters(&cache) {
        let Some(descriptor) = adapter.platform_credentials() else {
            continue;
        };
        let contributes = if collection == provider_config::COLLECTION_NAME {
            matches!(descriptor.backing, PlatformCredentialBacking::ProviderOAuth { provider_slug }
                if source.get_str("slug") == Ok(provider_slug))
                && ["client_id_encrypted", "client_secret_encrypted"]
                    .iter()
                    .any(|field| source.get(*field).is_some_and(|value| value != &Bson::Null))
        } else if source.get_str("provider") == Ok(descriptor.provider) {
            source
                .get_document("secrets")
                .is_ok_and(|secrets| match descriptor.backing {
                    PlatformCredentialBacking::Stored => !secrets.is_empty(),
                    PlatformCredentialBacking::ProviderOAuth { .. } => {
                        descriptor.fields.iter().any(|field| {
                            !matches!(field.name, "client_id" | "client_secret")
                                && secrets.contains_key(field.name)
                        })
                    }
                })
        } else {
            false
        };
        if contributes {
            let mut sources = vec!["platform"];
            if adapter.platform_webhook()
                && matches!(
                    adapter.credential_resolution(),
                    super::channel_platform::CredentialResolution::OAuthConnection { .. }
                )
            {
                sources.push("connection");
            }
            consumers
                .push(doc! {"platform":adapter.platform_id(),"credential_source":{"$in":sources}});
        }
    }
    if consumers.is_empty() {
        return Ok(());
    }
    fence_registration(db, session).await?;
    if db
        .collection::<Document>(channel_bot::COLLECTION_NAME)
        .find_one(doc! {"$or":consumers})
        .projection(doc! {"_id":1})
        .session(session)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "Channel signing credential requires fingerprint preparation".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_bot_attached_after_the_migration_snapshot_conflicts_with_the_registration_fence() {
        let db = crate::test_utils::connect_transaction_test_database(
            "channel_signing_registration_race",
        )
        .await;
        let settings = db.collection::<Document>(platform_settings::COLLECTION_NAME);
        let id = doc! {"_id":platform_settings::PLATFORM_SETTINGS_ID};
        settings.insert_one(doc!{"_id":platform_settings::PLATFORM_SETTINGS_ID,"channel_bot_registration_revision":0_i64}).await.unwrap();
        let mut migrate = db.client().start_session().await.unwrap();
        migrate
            .start_transaction()
            .read_concern(mongodb::options::ReadConcern::snapshot())
            .await
            .unwrap();
        assert!(
            settings
                .find_one(id.clone())
                .session(&mut migrate)
                .await
                .unwrap()
                .is_some()
        );
        let mut register = db.client().start_session().await.unwrap();
        register.start_transaction().await.unwrap();
        settings
            .update_one(
                id,
                doc! {"$inc":{"channel_bot_registration_revision":1_i64}},
            )
            .session(&mut register)
            .await
            .unwrap();
        db.collection::<Document>(channel_bot::COLLECTION_NAME)
            .insert_one(doc! {"_id":"attached","platform":"x","credential_source":"connection"})
            .session(&mut register)
            .await
            .unwrap();
        register.commit_transaction().await.unwrap();
        let source = doc! {"slug":"twitter","client_secret_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![1]}};
        let result = guard(&db, &mut migrate, provider_config::COLLECTION_NAME, &source).await;
        assert!(
            matches!(result,Err(AppError::DatabaseError(e)) if e.contains_label("TransientTransactionError"))
        );
        migrate.abort_transaction().await.unwrap();
        let mut fresh = db.client().start_session().await.unwrap();
        fresh.start_transaction().await.unwrap();
        assert!(matches!(
            guard(&db, &mut fresh, provider_config::COLLECTION_NAME, &source).await,
            Err(AppError::Conflict(_))
        ));
        fresh.abort_transaction().await.unwrap();
    }
}
