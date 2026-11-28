//! The mixed-binary driver exchanges the exact stored delivery and expected byte digests.
use super::*;
use crate::{
    AppState,
    handlers::social_auth,
    models::{refresh_token, session},
    services::token_service,
    test_utils,
};
use axum::{Json, extract::State, response::IntoResponse};
use bson::Document;
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};

const DELIVERIES: &str = MobileSocialDelivery::COLLECTION_NAME;

#[derive(Serialize, Deserialize)]
struct Fixture {
    delivery: Document,
    code: String,
    verifier: String,
    plaintext_hash: String,
    response_hash: String,
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

async fn state(writes: bool, legacy: bool) -> AppState {
    let db = test_utils::connect_transaction_test_database("mobile_social_record").await;
    let mut app = test_utils::test_app_state(db);
    let mut keys = test_utils::test_encryption_keys();
    keys.set_contextual_record_writes(writes);
    keys.set_allow_legacy_record_reads(legacy);
    app.encryption_keys = Arc::new(keys);
    app
}

impl Fixture {
    async fn issue(app: &AppState) -> Self {
        let user = Uuid::new_v4().to_string();
        let verifier = oauth_flow::generate_code_verifier();
        let challenge = oauth_flow::generate_code_challenge(&verifier);
        let prepared = token_service::prepare_session_tokens(
            &app.db,
            &app.config,
            &app.jwt_keys,
            &user,
            None,
            None,
        )
        .await
        .unwrap();
        let tokens = TokenDelivery {
            access_token: Zeroizing::new(prepared.tokens.access_token.clone()),
            refresh_token: Zeroizing::new(prepared.tokens.refresh_token.clone()),
            expires_in: prepared.tokens.access_expires_in,
        };
        #[derive(Serialize)]
        struct Expected<'a> {
            tokens: &'a TokenDelivery,
            rows: &'a PreparedSessionRows,
        }
        let plaintext_hash = digest(&Zeroizing::new(
            bson::to_vec(&Expected {
                tokens: &tokens,
                rows: &prepared.rows,
            })
            .unwrap(),
        ));
        let response_hash = digest(&Zeroizing::new(serde_json::to_vec(&tokens).unwrap()));
        let code = issue_code(&app.db, &app.encryption_keys, prepared, &user, &challenge)
            .await
            .unwrap();
        let delivery = app
            .db
            .collection::<Document>(DELIVERIES)
            .find_one(doc! {"code_hash": hash_token(&code)})
            .await
            .unwrap()
            .unwrap();
        let hash = hash_token(&code);
        let stored: MobileSocialDelivery = bson::from_document(delivery.clone()).unwrap();
        if app.encryption_keys.contextual_record_writes() {
            assert_eq!(delivery.get_i32("ciphertext_context_version"), Ok(3));
        } else {
            assert!(!delivery.contains_key("ciphertext_context_version"));
        }
        let plaintext = record::Binding::new(&stored.id, &user, &hash, &challenge)
            .open(&app.encryption_keys, &stored)
            .await
            .unwrap();
        assert_eq!(digest(&plaintext), plaintext_hash);
        Self {
            delivery,
            code,
            verifier,
            plaintext_hash,
            response_hash,
        }
    }

    fn write(&self, path: &Path) {
        std::fs::write(path, bson::to_vec(self).unwrap()).unwrap();
    }

    async fn restore(app: &AppState, path: &Path) -> Self {
        let fixture: Self = bson::from_slice(&std::fs::read(path).unwrap()).unwrap();
        app.db
            .collection::<Document>(DELIVERIES)
            .insert_one(&fixture.delivery)
            .await
            .unwrap();
        fixture
    }

    async fn claim(&self, app: &AppState) -> AppResult<String> {
        let body = serde_json::from_value(
            serde_json::json!({"code": &self.code, "code_verifier": &self.verifier}),
        )
        .unwrap();
        let response = social_auth::redeem_mobile_code(State(app.clone()), Json(body))
            .await?
            .into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response.headers()[axum::http::header::CACHE_CONTROL],
            "no-store"
        );
        Ok(digest(
            &axum::body::to_bytes(response.into_body(), 256 * 1024)
                .await
                .unwrap(),
        ))
    }
}

async fn counts(app: &AppState) -> (u64, u64, u64) {
    let count = async |name| {
        app.db
            .collection::<Document>(name)
            .count_documents(doc! {})
            .await
            .unwrap()
    };
    (
        count(DELIVERIES).await,
        count(session::COLLECTION_NAME).await,
        count(refresh_token::COLLECTION_NAME).await,
    )
}

#[tokio::test]
async fn mixed_pkce_delivery() {
    let app = state(
        std::env::var("CONTEXTUAL_RECORD_WRITES").as_deref() == Ok("true"),
        std::env::var("ALLOW_LEGACY_RECORD_READS").as_deref() != Ok("false"),
    )
    .await;
    let step = std::env::var("NYXID_RECORD_FIXTURE_STEP").unwrap_or_else(|_| "roundtrip".into());
    let file = std::env::var_os("NYXID_RECORD_FIXTURE_FILE");
    let fixture = match step.as_str() {
        "write" | "roundtrip" => {
            let fixture = Fixture::issue(&app).await;
            if let Some(file) = file {
                fixture.write(file.as_ref());
                return;
            }
            fixture
        }
        "read" | "reject" => Fixture::restore(&app, file.as_ref().unwrap().as_ref()).await,
        _ => panic!("unknown mixed-binary step"),
    };
    let result = fixture.claim(&app).await;
    if step == "reject" {
        assert!(result.is_err());
        assert_eq!(counts(&app).await, (1, 0, 0));
        assert!(
            app.db
                .collection::<Document>(DELIVERIES)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap()
                == fixture.delivery
        );
    } else {
        assert_eq!(result.unwrap(), fixture.response_hash);
        assert!(fixture.claim(&app).await.is_err());
        assert_eq!(counts(&app).await, (0, 1, 1));
    }
}

// Candidate storage tests
#[tokio::test]
async fn rewrap_preserves_bson_and_http_response() {
    let app = state(false, true).await;
    let file = std::env::var_os("NYXID_RECORD_FIXTURE_FILE");
    let mut fixture = if let Some(file) = &file {
        Fixture::restore(&app, file.as_ref()).await
    } else {
        Fixture::issue(&app).await
    };
    let keys = test_utils::test_encryption_keys();
    let before = &fixture.delivery;
    assert!(!before.contains_key("ciphertext_context_version"));
    let hash = hash_token(&fixture.code);
    let challenge = oauth_flow::generate_code_challenge(&fixture.verifier);
    let user = before.get_str("user_id").unwrap();
    let stored: MobileSocialDelivery = bson::from_document(before.clone()).unwrap();
    let original = record::Binding::new(&stored.id, user, &hash, &challenge)
        .open(&keys, &stored)
        .await
        .unwrap();
    assert_eq!(digest(&original), fixture.plaintext_hash);
    assert!(
        record::migrate(&app.db, &app.encryption_keys, before)
            .await
            .is_err()
    );
    assert!(record::migrate(&app.db, &keys, before).await.unwrap());
    assert!(!record::migrate(&app.db, &keys, before).await.unwrap());
    let rows = app.db.collection::<Document>(DELIVERIES);
    let current = rows.find_one(doc! {}).await.unwrap().unwrap();
    let mut metadata = current.clone();
    assert_eq!(
        metadata.remove("ciphertext_context_version"),
        Some(bson::Bson::Int32(3))
    );
    metadata.insert("ciphertext", before.get("ciphertext").unwrap().clone());
    assert!(metadata == *before);
    let path = ["ciphertext", &hash, &challenge];
    let context = crate::crypto::record_field::RecordContext::new(
        DELIVERIES,
        current.get_str("_id").unwrap(),
        user,
        &path,
    )
    .unwrap();
    let plain = keys
        .decrypt_record(current.get_binary_generic("ciphertext").unwrap(), &context)
        .await
        .unwrap();
    assert!(original.as_slice() == plain.as_slice());
    record::verify(&keys, &current).await.unwrap();
    assert!(!record::migrate(&app.db, &keys, &current).await.unwrap());
    assert_eq!(counts(&app).await, (1, 0, 0));
    fixture.delivery = current;
    if let Some(file) = file {
        fixture.write(file.as_ref());
    }
    assert_eq!(fixture.claim(&app).await.unwrap(), fixture.response_hash);
    assert_eq!(counts(&app).await, (0, 1, 1));
}

#[tokio::test]
async fn rejects_wrong_pkce_user_row_and_field() {
    let app = state(true, true).await;
    let fixture = Fixture::issue(&app).await;
    let rows = app.db.collection::<Document>(DELIVERIES);
    assert!(
        redeem(
            &app.db,
            &app.encryption_keys,
            &fixture.code,
            &"w".repeat(43)
        )
        .await
        .is_err()
    );
    let original = &fixture.delivery;
    let user = original.get_str("user_id").unwrap();
    let id = original.get_str("_id").unwrap();
    let hash = original.get_str("code_hash").unwrap();
    let challenge = original.get_str("code_challenge").unwrap();
    let keys = &app.encryption_keys;
    let path = ["ciphertext", hash, challenge];
    let context =
        crate::crypto::record_field::RecordContext::new(DELIVERIES, id, user, &path).unwrap();
    let plain = keys
        .decrypt_record(original.get_binary_generic("ciphertext").unwrap(), &context)
        .await
        .unwrap();
    let wrong_field = ["other_field", hash, challenge];
    let other = crate::crypto::record_field::RecordContext::new(DELIVERIES, id, user, &wrong_field)
        .unwrap();
    let misplaced = bson::Binary {
        subtype: bson::spec::BinarySubtype::Generic,
        bytes: keys.encrypt_record(&plain, &other).await.unwrap(),
    };
    for change in [
        doc! {"user_id": Uuid::new_v4().to_string()},
        doc! {"_id": Uuid::new_v4().to_string()},
        doc! {"code_hash": hash_token("other code")},
        doc! {"code_challenge": oauth_flow::generate_code_challenge(&"x".repeat(43))},
        doc! {"ciphertext": misplaced},
        doc! {"ciphertext_context_version": 4},
        doc! {"ciphertext_context_version": bson::Bson::Null},
    ] {
        let mut swapped = original.clone();
        swapped.extend(change);
        rows.delete_many(doc! {}).await.unwrap();
        rows.insert_one(&swapped).await.unwrap();
        assert!(record::verify(keys, &swapped).await.is_err());
        assert!(record::migrate(&app.db, keys, &swapped).await.is_err());
        assert!(fixture.claim(&app).await.is_err());
        assert_eq!(counts(&app).await, (1, 0, 0));
    }
    // Even a well-formed envelope must contain prepared rows for this delivery's owner.
    let mut wrong: Document = bson::from_slice(&plain).unwrap();
    wrong
        .get_document_mut("rows")
        .unwrap()
        .get_document_mut("refresh")
        .unwrap()
        .insert("user_id", Uuid::new_v4().to_string());
    let malformed = Zeroizing::new(bson::to_vec(&wrong).unwrap());
    let bytes = keys.encrypt_record(&malformed, &context).await.unwrap();
    let mut swapped = original.clone();
    swapped.insert(
        "ciphertext",
        bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes,
        },
    );
    rows.replace_one(doc! {}, &swapped).await.unwrap();
    assert!(record::verify(keys, &swapped).await.is_err());
    assert!(fixture.claim(&app).await.is_err());
    assert_eq!(counts(&app).await, (1, 0, 0));
}

#[tokio::test]
async fn expired_delivery_rewrap_preserves_deadline_and_ttl_delete_wins() {
    let app = state(false, true).await;
    let fixture = Fixture::issue(&app).await;
    let rows = app.db.collection::<Document>(DELIVERIES);
    rows.update_one(
        doc! {},
        doc! {"$set": {"expires_at": bson::DateTime::from_millis(0)}},
    )
    .await
    .unwrap();
    let keys = test_utils::test_encryption_keys();
    assert!(
        !record::migrate(&app.db, &keys, &fixture.delivery)
            .await
            .unwrap()
    );
    let retained = rows.find_one(doc! {}).await.unwrap().unwrap();
    assert!(fixture.claim(&app).await.is_err());
    assert!(record::migrate(&app.db, &keys, &retained).await.unwrap());
    let current = rows.find_one(doc! {}).await.unwrap().unwrap();
    assert_eq!(
        current.get_datetime("expires_at"),
        retained.get_datetime("expires_at")
    );
    record::verify(&keys, &current).await.unwrap();
    assert!(fixture.claim(&app).await.is_err());

    let rotated = EncryptionKeys::with_provider(Arc::new(
        crate::crypto::local_key_provider::LocalKeyProvider::new([0x42; 32], Some([0x11; 32])),
    ));
    assert!(record::verify(&rotated, &current).await.is_err());
    assert!(record::migrate(&app.db, &rotated, &current).await.unwrap());
    let winner = rows.find_one(doc! {}).await.unwrap().unwrap();
    record::verify(&rotated, &winner).await.unwrap();
    rows.delete_one(doc! {"expires_at": {"$lte": bson::DateTime::now()}})
        .await
        .unwrap();
    assert!(!record::migrate(&app.db, &keys, &retained).await.unwrap());
    assert_eq!(counts(&app).await, (0, 0, 0));
}

#[tokio::test]
async fn two_redeemers_during_rewrap_install_one_session_in_either_order() {
    use crate::crypto::{
        key_provider::{KeyProvider, WrappedKey},
        local_key_provider::LocalKeyProvider,
    };
    use tokio::sync::Barrier;
    #[derive(Debug)]
    struct PausedRead {
        local: LocalKeyProvider,
        reads_to_pause: std::sync::atomic::AtomicUsize,
        entered: Barrier,
        resume: Barrier,
    }
    #[async_trait::async_trait]
    impl KeyProvider for PausedRead {
        async fn wrap_dek(&self, key: &[u8]) -> AppResult<WrappedKey> {
            self.local.wrap_dek(key).await
        }
        async fn unwrap_dek(&self, key: &WrappedKey) -> AppResult<Zeroizing<Vec<u8>>> {
            use std::sync::atomic::Ordering::SeqCst;
            if self
                .reads_to_pause
                .fetch_update(SeqCst, SeqCst, |left| left.checked_sub(1))
                .is_ok()
            {
                self.entered.wait().await;
                self.resume.wait().await;
            }
            self.local.unwrap_dek(key).await
        }
        fn current_key_id(&self) -> u8 {
            self.local.current_key_id()
        }
        fn has_key_id(&self, id: u8) -> bool {
            self.local.has_key_id(id)
        }
        fn has_previous_key(&self) -> bool {
            false
        }
    }

    for rewrap_first in [true, false] {
        let mut app = state(false, true).await;
        let fixture = Fixture::issue(&app).await;
        let parties = if rewrap_first { 3 } else { 2 };
        let paused = Arc::new(PausedRead {
            local: LocalKeyProvider::new([0x11; 32], None),
            reads_to_pause: (parties - 1).into(),
            entered: Barrier::new(parties),
            resume: Barrier::new(parties),
        });
        let mut migration_keys = Arc::new(test_utils::test_encryption_keys());
        let gated_keys = Arc::new(EncryptionKeys::with_provider(paused.clone()));
        if rewrap_first {
            app.encryption_keys = gated_keys;
        } else {
            migration_keys = gated_keys;
        }
        let migrate = || record::migrate(&app.db, &migration_keys, &fixture.delivery);
        let redeem_twice = || async {
            let (first, second) = tokio::join!(fixture.claim(&app), fixture.claim(&app));
            match (first, second) {
                (Ok(hash), Err(AppError::ValidationError(_)))
                | (Err(AppError::ValidationError(_)), Ok(hash)) => {
                    assert_eq!(hash, fixture.response_hash)
                }
                _ => panic!("expected one committed delivery"),
            }
        };
        let race = async {
            if rewrap_first {
                let publish = async {
                    paused.entered.wait().await;
                    assert!(migrate().await.unwrap());
                    paused.resume.wait().await;
                };
                tokio::join!(publish, redeem_twice());
            } else {
                let claims = async {
                    paused.entered.wait().await;
                    redeem_twice().await;
                    paused.resume.wait().await;
                };
                let (result, ()) = tokio::join!(migrate(), claims);
                assert!(!result.unwrap());
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(30), Box::pin(race))
            .await
            .unwrap();
        assert_eq!(counts(&app).await, (0, 1, 1));
    }
}
