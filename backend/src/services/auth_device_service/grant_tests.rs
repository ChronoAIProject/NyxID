use super::*;
use crate::{
    crypto::{
        key_provider::{KeyProvider, WrappedKey},
        local_key_provider::LocalKeyProvider,
    },
    models::{
        api_key::{ApiKey, COLLECTION_NAME as API_KEYS},
        api_key_credential::{ApiKeyCredential, COLLECTION_NAME as CHILDREN},
        session::{COLLECTION_NAME as SESSIONS, Session},
        user::{COLLECTION_NAME as USERS, User, UserType},
    },
    services::auth_agent_key_login_service::Selection,
    test_utils::{connect_transaction_test_database, test_app_state, test_user},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const KEY: &[u8] = b"device-grant-fixture-key";

async fn fixture(name: &str) -> (crate::AppState, String) {
    let db = connect_transaction_test_database(name).await;
    crate::db::ensure_indexes(&db).await.unwrap();
    let user = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&user, UserType::Person))
        .await
        .unwrap();
    (test_app_state(db), user)
}
fn input(user: &str, code: &str) -> ApproveInput {
    ApproveInput {
        user_id: user.into(),
        user_code: code.into(),
        approver_ip: None,
        approver_user_agent: None,
    }
}
fn selection() -> Selection {
    serde_json::from_value(
        serde_json::json!({"kind":"new", "name":"Fixture restricted", "scopes":"proxy"}),
    )
    .unwrap()
}

#[tokio::test]
async fn abandoned_new_parent_survives_reuse_and_concurrent_issuance() {
    let (state, actor) = fixture("device_reused_parent_cleanup").await;
    for concurrent in [false, true] {
        let first = initiate_v2(&state.db, KEY, InitiateInput::default())
            .await
            .unwrap();
        approve_with_agent_key(
            &state.db,
            &state.encryption_keys,
            KEY,
            input(&actor, &first.user_code),
            selection(),
            None,
        )
        .await
        .unwrap();
        let rows = collection_for_protocol(&state.db, true);
        let row = rows
            .find_one(doc! {"device_code_hmac": hmac_hex(KEY, first.device_code.as_bytes())})
            .await
            .unwrap()
            .unwrap();
        let grant = row.agent_key_grant.unwrap();
        let second = initiate_v2(&state.db, KEY, InitiateInput::default())
            .await
            .unwrap();
        let approve = approve_with_agent_key(
            &state.db,
            &state.encryption_keys,
            KEY,
            input(&actor, &second.user_code),
            Selection::Existing {
                api_key_id: grant.api_key_id.clone(),
            },
            None,
        );
        rows.update_one(doc! {"_id": &row.id}, doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}}).await.unwrap();
        let approved = if concurrent {
            let (approved, swept) = tokio::join!(approve, grants::sweep_expired(&state.db));
            swept.unwrap();
            approved
        } else {
            let approved = approve.await;
            grants::sweep_expired(&state.db).await.unwrap();
            approved
        };
        if !concurrent {
            assert!(approved.is_ok());
        }
        if approved.is_ok() {
            let delivery =
                poll_agent_key(&state.db, &state.encryption_keys, KEY, &second.device_code)
                    .await
                    .unwrap();
            assert!(
                crate::services::key_service::validate_api_key(&state.db, &delivery.credential)
                    .await
                    .is_ok()
            );
        } else {
            assert_eq!(
                state
                    .db
                    .collection::<ApiKeyCredential>(CHILDREN)
                    .count_documents(doc! {"api_key_id": &grant.api_key_id, "is_active": true})
                    .await
                    .unwrap(),
                0
            );
        }
        let abandoned = state
            .db
            .collection::<ApiKeyCredential>(CHILDREN)
            .find_one(doc! {"_id": &grant.credential_id})
            .await
            .unwrap()
            .unwrap();
        assert!(!abandoned.is_active);
    }
}

#[tokio::test]
async fn v2_grants_are_invisible_to_legacy_poll_and_expiry_indexes() {
    let (state, actor) = fixture("device_rolling_protocol").await;
    let legacy = initiate(&state.db, KEY, InitiateInput::default())
        .await
        .unwrap();
    assert_eq!(normalize_user_code(&legacy.user_code).unwrap().len(), 8);
    assert!(
        options(&state.db, KEY, &actor, &legacy.user_code)
            .await
            .is_err()
    );
    assert!(
        approve_with_agent_key(
            &state.db,
            &state.encryption_keys,
            KEY,
            input(&actor, &legacy.user_code),
            selection(),
            None
        )
        .await
        .is_err()
    );
    let request = initiate_v2(&state.db, KEY, InitiateInput::default())
        .await
        .unwrap();
    assert!(request.user_code.starts_with("2-"));
    assert!(request.device_code.starts_with("nyx_adc2_"));
    approve_with_agent_key(
        &state.db,
        &state.encryption_keys,
        KEY,
        input(&actor, &request.user_code),
        selection(),
        None,
    )
    .await
    .unwrap();
    let legacy_codes = collection(&state.db);
    // This is the old replica's delivery filter and old startup TTL definition.
    assert!(legacy_codes.find_one_and_update(doc! {"device_code_hmac": hmac_hex(KEY, request.device_code.as_bytes()), "status":"approved"},
        doc! {"$set":{"status":"delivered"}}).await.unwrap().is_none());
    legacy_codes
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"expires_at":1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    let mut indexes = collection_for_protocol(&state.db, true)
        .list_indexes()
        .await
        .unwrap();
    use futures::TryStreamExt;
    while let Some(index) = indexes.try_next().await.unwrap() {
        assert!(
            !(index.keys == doc! {"expires_at":1}
                && index
                    .options
                    .is_some_and(|options| options.expire_after.is_some()))
        );
    }
    let delivered = poll_agent_key(&state.db, &state.encryption_keys, KEY, &request.device_code)
        .await
        .unwrap();
    assert!(delivered.credential.starts_with("nyxid_ag_"));
    approve(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &state.encryption_keys,
        KEY,
        input(&actor, &legacy.user_code),
    )
    .await
    .unwrap();
    assert!(matches!(
        poll_and_prepare(&state.db, &state.encryption_keys, KEY, &legacy.device_code)
            .await
            .unwrap(),
        PollClaim::Account(_)
    ));
}

#[tokio::test]
async fn account_and_agent_approval_race_commits_only_one_grant() {
    let (state, actor) = fixture("device_grant_race").await;
    let request = initiate_v2(
        &state.db,
        KEY,
        InitiateInput {
            requested_profile: Some("fixture-profile".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (account, restricted) = tokio::join!(
        approve(
            &state.db,
            &state.config,
            &state.jwt_keys,
            &state.encryption_keys,
            KEY,
            input(&actor, &request.user_code)
        ),
        approve_with_agent_key(
            &state.db,
            &state.encryption_keys,
            KEY,
            input(&actor, &request.user_code),
            selection(),
            None
        )
    );
    assert_ne!(account.is_ok(), restricted.is_ok());
    let sessions = state
        .db
        .collection::<Session>(SESSIONS)
        .count_documents(doc! {})
        .await
        .unwrap();
    let children = state
        .db
        .collection::<ApiKeyCredential>(CHILDREN)
        .count_documents(doc! {})
        .await
        .unwrap();
    assert_eq!(sessions + children, 1);
    assert_eq!(
        state
            .db
            .collection::<ApiKey>(API_KEYS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        children
    );
    let (a, b) = tokio::join!(
        poll_and_prepare(&state.db, &state.encryption_keys, KEY, &request.device_code),
        poll_and_prepare(&state.db, &state.encryption_keys, KEY, &request.device_code)
    );
    if account.is_ok() {
        assert_eq!(
            [a.unwrap(), b.unwrap()]
                .iter()
                .filter(|v| matches!(v, PollClaim::Account(_)))
                .count(),
            1
        );
    } else {
        let (a, b) = tokio::join!(
            poll_agent_key(&state.db, &state.encryption_keys, KEY, &request.device_code),
            poll_agent_key(&state.db, &state.encryption_keys, KEY, &request.device_code)
        );
        assert_ne!(a.is_ok(), b.is_ok());
        let delivery = a.or(b).unwrap();
        assert!(delivery.label.contains("fixture-profile"));
    }
}

#[derive(Debug)]
struct FallibleProvider {
    local: LocalKeyProvider,
    fail: AtomicBool,
}
#[async_trait::async_trait]
impl KeyProvider for FallibleProvider {
    async fn wrap_dek(&self, key: &[u8]) -> AppResult<WrappedKey> {
        self.local.wrap_dek(key).await
    }
    async fn unwrap_dek(&self, key: &WrappedKey) -> AppResult<Zeroizing<Vec<u8>>> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(AppError::Internal("fixture KMS unavailable".into()));
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

#[derive(Debug)]
struct PausedProvider {
    local: LocalKeyProvider,
    pause: AtomicBool,
    entered: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl KeyProvider for PausedProvider {
    async fn wrap_dek(&self, key: &[u8]) -> AppResult<WrappedKey> {
        self.local.wrap_dek(key).await
    }
    async fn unwrap_dek(&self, key: &WrappedKey) -> AppResult<Zeroizing<Vec<u8>>> {
        if self.pause.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.resume.notified().await;
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

#[tokio::test]
async fn expiry_during_decryption_reports_expired_for_cli_and_browser() {
    let (state, actor) = fixture("device_decrypt_expiry").await;
    let provider = Arc::new(PausedProvider {
        local: LocalKeyProvider::new([42; 32], None),
        pause: AtomicBool::new(false),
        entered: Default::default(),
        resume: Default::default(),
    });
    let encryption = EncryptionKeys::with_provider(provider.clone());
    for browser in [false, true] {
        let request = initiate_v2(&state.db, KEY, InitiateInput::default())
            .await
            .unwrap();
        approve(
            &state.db,
            &state.config,
            &state.jwt_keys,
            &encryption,
            KEY,
            input(&actor, &request.user_code),
        )
        .await
        .unwrap();
        provider.pause.store(true, Ordering::SeqCst);
        let poll = async {
            if browser {
                poll_for_browser(
                    &state.db,
                    &encryption,
                    KEY,
                    &request.device_code,
                    "fixture-cookie",
                    "192.0.2.1",
                    None,
                )
                .await
            } else {
                poll_and_prepare(&state.db, &encryption, KEY, &request.device_code).await
            }
        };
        let expire = async {
            provider.entered.notified().await;
            collection_for_protocol(&state.db, true).update_one(doc! {"device_code_hmac": hmac_hex(KEY, request.device_code.as_bytes())},
                doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}}).await.unwrap();
            provider.resume.notify_one();
        };
        let (result, ()) = tokio::join!(poll, expire);
        assert!(matches!(result.unwrap(), PollClaim::Expired));
        grants::sweep_expired(&state.db).await.unwrap();
        assert_eq!(
            state
                .db
                .collection::<Session>(SESSIONS)
                .count_documents(doc! {"revoked": false})
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn failed_kms_unwrap_keeps_account_delivery_retryable_and_expiry_cleans_up() {
    let (state, actor) = fixture("device_grant_kms").await;
    let provider = Arc::new(FallibleProvider {
        local: LocalKeyProvider::new([41; 32], None),
        fail: AtomicBool::new(false),
    });
    let encryption = EncryptionKeys::with_provider(provider.clone());
    for deliver in [true, false] {
        let request = initiate_v2(&state.db, KEY, InitiateInput::default())
            .await
            .unwrap();
        approve(
            &state.db,
            &state.config,
            &state.jwt_keys,
            &encryption,
            KEY,
            input(&actor, &request.user_code),
        )
        .await
        .unwrap();
        provider.fail.store(true, Ordering::SeqCst);
        assert!(
            poll_and_prepare(&state.db, &encryption, KEY, &request.device_code)
                .await
                .is_err()
        );
        assert!(
            poll_for_browser(
                &state.db,
                &encryption,
                KEY,
                &request.device_code,
                "cookie-hash",
                "192.0.2.1",
                None
            )
            .await
            .is_err()
        );
        let row = collection_for_protocol(&state.db, true)
            .find_one(doc! {"device_code_hmac": hmac_hex(KEY, request.device_code.as_bytes())})
            .await
            .unwrap()
            .unwrap();
        assert!(
            row.status == AuthDeviceCodeStatus::Approved
                && row.delivery_access_token_encrypted.is_some()
        );
        provider.fail.store(false, Ordering::SeqCst);
        if deliver {
            assert!(matches!(
                poll_and_prepare(&state.db, &encryption, KEY, &request.device_code)
                    .await
                    .unwrap(),
                PollClaim::Account(_)
            ));
        } else {
            collection_for_protocol(&state.db, true).update_one(doc! {"_id": &row.id}, doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}}).await.unwrap();
            grants::sweep_expired(&state.db).await.unwrap();
            let session = state
                .db
                .collection::<Session>(SESSIONS)
                .find_one(doc! {"_id": row.approved_session_id})
                .await
                .unwrap()
                .unwrap();
            assert!(session.revoked);
            let row = collection_for_protocol(&state.db, true)
                .find_one(doc! {"_id": row.id})
                .await
                .unwrap()
                .unwrap();
            assert!(row.purge_at.is_some() && row.delivery_access_token_encrypted.is_none());
        }
    }
}

#[tokio::test]
async fn browser_session_insert_failure_rolls_back_delivery_and_revocation() {
    let (state, actor) = fixture("device_browser_rollback").await;
    let request = initiate_v2(&state.db, KEY, InitiateInput::default())
        .await
        .unwrap();
    approve(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &state.encryption_keys,
        KEY,
        input(&actor, &request.user_code),
    )
    .await
    .unwrap();
    state
        .db
        .run_command(
            doc! {"collMod": SESSIONS, "validator": {"user_agent": {"$ne": "fixture-browser"}}},
        )
        .await
        .unwrap();
    assert!(
        poll_for_browser(
            &state.db,
            &state.encryption_keys,
            KEY,
            &request.device_code,
            "hash",
            "192.0.2.1",
            Some("fixture-browser")
        )
        .await
        .is_err()
    );
    assert_eq!(
        state
            .db
            .collection::<Session>(SESSIONS)
            .count_documents(doc! {"revoked": false})
            .await
            .unwrap(),
        1
    );
    let row = collection_for_protocol(&state.db, true)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert!(
        row.status == AuthDeviceCodeStatus::Approved
            && row.delivery_refresh_token_encrypted.is_some()
    );
    state
        .db
        .run_command(doc! {"collMod": SESSIONS, "validator": {}})
        .await
        .unwrap();
    assert!(matches!(
        poll_for_browser(
            &state.db,
            &state.encryption_keys,
            KEY,
            &request.device_code,
            "hash",
            "192.0.2.1",
            Some("fixture-browser")
        )
        .await
        .unwrap(),
        PollClaim::Account(_)
    ));
    assert_eq!(
        state
            .db
            .collection::<Session>(SESSIONS)
            .count_documents(doc! {"revoked": false})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        state
            .db
            .collection::<Session>(SESSIONS)
            .count_documents(doc! {"revoked": true})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn retained_terminal_outcomes_precede_expiry_and_codes_are_not_reused() {
    let (state, actor) = fixture("device_terminal_code_reuse").await;
    let request =
        initiate_with_user_code_generator(&state.db, KEY, InitiateInput::default(), || {
            "2ABCDEFGH".into()
        })
        .await
        .unwrap();
    deny(
        &state.db,
        KEY,
        DenyInput {
            user_id: actor.clone(),
            user_code: request.user_code.clone(),
            denier_ip: None,
            denier_user_agent: None,
        },
    )
    .await
    .unwrap();
    collection_for_protocol(&state.db, true).update_one(doc! {}, doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}}).await.unwrap();
    assert!(matches!(
        approve(
            &state.db,
            &state.config,
            &state.jwt_keys,
            &state.encryption_keys,
            KEY,
            input(&actor, &request.user_code)
        )
        .await,
        Err(AppError::AuthDeviceCodeDenied)
    ));
    assert!(matches!(
        options(&state.db, KEY, &actor, &request.user_code).await,
        Err(AppError::AuthDeviceCodeDenied)
    ));
    let mut calls = 0;
    let next = initiate_with_user_code_generator(&state.db, KEY, InitiateInput::default(), || {
        calls += 1;
        if calls == 1 {
            "2ABCDEFGH".into()
        } else {
            "212345678".into()
        }
    })
    .await
    .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(next.user_code, "2-1234-5678");
}
