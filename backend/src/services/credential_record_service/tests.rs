use super::*;
use bson::{Bson, Document, doc};

#[tokio::test]
async fn legacy_array_cas_strict_readiness_and_rewrap_preserve_authority() {
    let db = crate::test_utils::connect_test_database("credential_record_exact_source")
        .await
        .unwrap();
    let keys = crate::test_utils::test_encryption_keys();
    let owner = uuid::Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &owner,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    for collection in [
        crate::models::provider_config::COLLECTION_NAME,
        crate::models::user_provider_credentials::COLLECTION_NAME,
        crate::models::user_provider_token::COLLECTION_NAME,
        crate::models::user_api_key::COLLECTION_NAME,
    ] {
        let mut source = doc! {"_id":uuid::Uuid::new_v4().to_string(),"user_id":&owner,"credential_epoch":17_i64,"state_version":9_i64,"status":"active","provider_config_id":"provider","connection_id":"connection","opaque_metadata_id":"preserved-metadata","updated_at":bson::DateTime::now()};
        for field in fields(collection).unwrap() {
            let encrypted = keys
                .encrypt(format!("distinctive-material-{field}").as_bytes())
                .await
                .unwrap();
            source.insert(
                *field,
                Bson::Array(
                    encrypted
                        .into_iter()
                        .map(|v| Bson::Int32(i32::from(v)))
                        .collect(),
                ),
            );
        }
        db.collection::<Document>(collection)
            .insert_one(&source)
            .await
            .unwrap();
        assert!(
            verify_current_snapshot(&keys, collection, &source)
                .await
                .is_err()
        );
        assert!(
            migrate_snapshot(&db, &keys, collection, &source)
                .await
                .unwrap()
        );
        assert!(
            !migrate_snapshot(&db, &keys, collection, &source)
                .await
                .unwrap()
        );
        let migrated = db
            .collection::<Document>(collection)
            .find_one(doc! {"_id":source.get("_id").unwrap()})
            .await
            .unwrap()
            .unwrap();
        verify_current_snapshot(&keys, collection, &migrated)
            .await
            .unwrap();
        let owner_before = db
            .collection::<Document>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! {"_id":&owner})
            .await
            .unwrap();
        assert!(
            !migrate_snapshot(&db, &keys, collection, &migrated)
                .await
                .unwrap()
        );
        assert_eq!(
            db.collection::<Document>(collection)
                .find_one(doc! {"_id":source.get("_id").unwrap()})
                .await
                .unwrap()
                .as_ref(),
            Some(&migrated)
        );
        assert_eq!(
            db.collection::<Document>(crate::models::user::COLLECTION_NAME)
                .find_one(doc! {"_id":&owner})
                .await
                .unwrap(),
            owner_before
        );
        for field in [
            "user_id",
            "credential_epoch",
            "state_version",
            "status",
            "provider_config_id",
            "connection_id",
            "opaque_metadata_id",
            "updated_at",
        ] {
            assert_eq!(migrated.get(field), source.get(field));
        }
        for field in fields(collection).unwrap() {
            let encrypted = stored_bytes(migrated.get(*field).unwrap())
                .unwrap()
                .unwrap();
            let plain = decrypt_stored_field(&keys, collection, &migrated, field, &encrypted)
                .await
                .unwrap();
            assert_eq!(
                plain.as_slice(),
                format!("distinctive-material-{field}").as_bytes()
            );
            let mut swapped = migrated.clone();
            swapped.insert("_id", uuid::Uuid::new_v4().to_string());
            assert!(
                decrypt_stored_field(&keys, collection, &swapped, field, &encrypted)
                    .await
                    .is_err()
            );
        }
        let mut malformed = migrated.clone();
        let field = fields(collection).unwrap()[0];
        let (id, bound_owner) = storage_identity(collection, &migrated).unwrap();
        let invalid = keys
            .encrypt_record(
                &[0xff, 0xfe],
                &RecordContext::new(collection, id, bound_owner, &[field]).unwrap(),
            )
            .await
            .unwrap();
        malformed.insert(
            field,
            Bson::Binary(bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: invalid,
            }),
        );
        assert!(
            verify_current_snapshot(&keys, collection, &malformed)
                .await
                .is_err()
        );
    }
    db.drop().await.unwrap();
}

#[derive(Debug)]
struct HoldFirstWrap {
    inner: crate::crypto::local_key_provider::LocalKeyProvider,
    held: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl crate::crypto::key_provider::KeyProvider for HoldFirstWrap {
    async fn wrap_dek(&self, key: &[u8]) -> AppResult<crate::crypto::key_provider::WrappedKey> {
        if !self.held.swap(true, std::sync::atomic::Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        self.inner.wrap_dek(key).await
    }
    async fn unwrap_dek(
        &self,
        wrapped: &crate::crypto::key_provider::WrappedKey,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        self.inner.unwrap_dek(wrapped).await
    }
    fn current_key_id(&self) -> u8 {
        self.inner.current_key_id()
    }
    fn has_key_id(&self, id: u8) -> bool {
        self.inner.has_key_id(id)
    }
    fn has_previous_key(&self) -> bool {
        false
    }
}

#[tokio::test]
async fn held_live_copy_and_startup_migration_cannot_install_after_source_refresh_disconnect_or_byo_change()
 {
    use crate::models::user_provider_token::COLLECTION_NAME as TOKENS;
    use crate::services::{oauth_flow, user_credentials_service, user_token_service};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    crate::test_utils::init_test_audit_chain_hmac_key();
    for (startup, winner) in [
        (false, "refresh"),
        (false, "disconnect"),
        (true, "refresh"),
        (true, "disconnect"),
        (true, "byo"),
    ] {
        let db = crate::test_utils::connect_test_database("credential_copy_source_fence")
            .await
            .unwrap();
        let keys = crate::test_utils::test_encryption_keys();
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600})))
            .mount(&server).await;
        let mut provider: ProviderConfig=bson::from_document(doc! {"_id":uuid::Uuid::new_v4().to_string(),"slug":"copy-source-fixture","name":"Copy source","provider_type":"oauth2","credential_mode":"both","is_active":true,"created_by":"admin","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),"token_url":format!("{}/token",server.uri())}).unwrap();
        provider.client_id_encrypted = Some(keys.encrypt(b"copy-client").await.unwrap());
        provider.client_secret_encrypted = Some(keys.encrypt(b"copy-client-secret").await.unwrap());
        db.collection::<ProviderConfig>(crate::models::provider_config::COLLECTION_NAME)
            .insert_one(&provider)
            .await
            .unwrap();
        if winner == "byo" {
            user_credentials_service::upsert_user_credentials(
                &db,
                &keys,
                &owner,
                &provider.id,
                "old-byo-client",
                Some("old-byo-secret"),
                None,
            )
            .await
            .unwrap();
        }
        let token: UserProviderToken=bson::from_document(doc! {"_id":uuid::Uuid::new_v4().to_string(),"user_id":&owner,"provider_config_id":&provider.id,"credential_user_id":if winner=="byo" {Bson::String(owner.clone())} else {Bson::Null},"token_type":"oauth2","status":"active","state_version":1_i64,"access_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:keys.encrypt(b"old-access").await.unwrap()},"refresh_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:keys.encrypt(b"old-refresh").await.unwrap()},"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).unwrap();
        db.collection::<UserProviderToken>(TOKENS)
            .insert_one(&token)
            .await
            .unwrap();
        let held = std::sync::Arc::new(HoldFirstWrap {
            inner: crate::crypto::local_key_provider::LocalKeyProvider::new([0x11; 32], None),
            held: Default::default(),
            entered: Default::default(),
            release: Default::default(),
        });
        let held_keys = EncryptionKeys::with_provider(held.clone());
        let task_db = db.clone();
        let expected = token.clone();
        let task_owner = owner.clone();
        let provider_id = provider.id.clone();
        let task = tokio::spawn(async move {
            if startup {
                crate::db::migrate_provider_tokens(&task_db, &held_keys)
                    .await
                    .map_err(|_| AppError::Conflict("Migration source changed".into()))
            } else {
                crate::services::user_api_key_service::create_api_key_from_provider_token(
                    &task_db,
                    &held_keys,
                    &task_owner,
                    "Source copy",
                    &provider_id,
                    &expected,
                )
                .await
                .map(drop)
            }
        });
        held.entered.notified().await;
        match winner {
            "refresh" => {
                oauth_flow::refresh_oauth_token(&db, &keys, &token)
                    .await
                    .unwrap();
            }
            "disconnect" => {
                user_token_service::claim_provider_token_by_id(&db, &owner, &token.id)
                    .await
                    .unwrap()
                    .unwrap();
            }
            "byo" => {
                user_credentials_service::upsert_user_credentials(
                    &db,
                    &keys,
                    &owner,
                    &provider.id,
                    "new-byo-client",
                    Some("new-byo-secret"),
                    None,
                )
                .await
                .unwrap();
            }
            _ => unreachable!(),
        }
        held.release.notify_one();
        let outcome = task.await.unwrap();
        if !startup {
            assert!(outcome.is_err());
        } else {
            outcome.unwrap();
        }
        for collection in [
            crate::models::user_api_key::COLLECTION_NAME,
            crate::models::user_endpoint::COLLECTION_NAME,
            crate::models::user_service::COLLECTION_NAME,
        ] {
            assert_eq!(
                db.collection::<Document>(collection)
                    .count_documents(doc! {"user_id":&owner})
                    .await
                    .unwrap(),
                0,
                "{startup}/{winner}: {collection}"
            );
        }
        if winner == "refresh" && !startup {
            let current = db
                .collection::<UserProviderToken>(TOKENS)
                .find_one(doc! {"_id":&token.id})
                .await
                .unwrap()
                .unwrap();
            let first = crate::services::user_api_key_service::create_api_key_from_provider_token(
                &db,
                &keys,
                &owner,
                "Current copy",
                &provider.id,
                &current,
            )
            .await
            .unwrap();
            let reused = crate::services::user_api_key_service::create_api_key_from_provider_token(
                &db,
                &keys,
                &owner,
                "Reuse",
                &provider.id,
                &current,
            )
            .await
            .unwrap();
            assert_eq!(first.id, reused.id);
            assert!(
                crate::services::user_api_key_service::create_api_key_from_provider_token(
                    &db,
                    &keys,
                    &owner,
                    "Stale reuse",
                    &provider.id,
                    &token
                )
                .await
                .is_err()
            );
            let plain = CredentialRecord::key(&first)
                .text(
                    &keys,
                    "access_token_encrypted",
                    first.access_token_encrypted.as_deref().unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(plain.as_str(), "new-access");
        }
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn startup_migration_skips_accepted_owner_without_starving_active_owner() {
    crate::test_utils::init_test_audit_chain_hmac_key();
    let db = crate::test_utils::connect_test_database("credential_migration_closed_owner")
        .await
        .unwrap();
    let keys = crate::test_utils::test_encryption_keys();
    crate::services::personal_erasure_service::ensure_indexes(&db)
        .await
        .unwrap();
    let mut token_ids = [
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    ];
    token_ids.sort();
    let mut owners = Vec::new();
    for token_id in token_ids {
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let now = bson::DateTime::now();
        db.collection::<Document>(crate::models::user_provider_token::COLLECTION_NAME).insert_one(doc! {
            "_id":token_id,"user_id":&owner,"provider_config_id":"missing-provider",
            "token_type":"api_key","status":"active","state_version":1_i64,
            "api_key_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:keys.encrypt(b"retained-api-key").await.unwrap()},
            "created_at":now,"updated_at":now,
        }).await.unwrap();
        owners.push(owner);
    }
    crate::services::personal_erasure_service::accept(&db, &owners[0], &owners[0], None)
        .await
        .unwrap();
    crate::db::migrate_provider_tokens(&db, &keys)
        .await
        .unwrap();
    for collection in [
        crate::models::user_endpoint::COLLECTION_NAME,
        crate::models::user_api_key::COLLECTION_NAME,
        crate::models::user_service::COLLECTION_NAME,
    ] {
        assert_eq!(
            db.collection::<Document>(collection)
                .count_documents(doc! {"user_id":&owners[0]})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<Document>(collection)
                .count_documents(doc! {"user_id":&owners[1]})
                .await
                .unwrap(),
            1
        );
    }
    assert!(
        db.collection::<Document>(crate::models::user_provider_token::COLLECTION_NAME)
            .find_one(doc! {"user_id":&owners[0]})
            .await
            .unwrap()
            .unwrap()
            .contains_key("api_key_encrypted")
    );
    db.drop().await.unwrap();
}
