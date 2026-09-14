use super::*;
use crate::crypto::token::hash_token;
use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgRole};
use crate::models::user::UserType;
use crate::test_utils::{
    connect_transaction_test_database, test_encryption_keys, test_membership, test_user,
};

const HMAC_KEY: &[u8] = b"agent-key-login-test-domain-key";

#[test]
fn credential_labels_distinguish_profiles_and_are_sanitized_and_bounded() {
    for (client, profile, expected) in [
        (Some("host\n"), Some("profile\r"), "host \u{00b7} profile"),
        (Some("host"), None, "host"),
        (None, Some("profile"), "profile"),
        (Some("\n"), Some("\r"), "NyxID CLI"),
        (None, None, "NyxID CLI"),
    ] {
        let context = LoginClientContext {
            client_label: client.map(str::to_owned),
            ..Default::default()
        };
        assert_eq!(credential_label(&context, profile), expected);
    }
    let context = LoginClientContext {
        client_label: Some("h".repeat(64)),
        ..Default::default()
    };
    let label = credential_label(&context, Some(&"p".repeat(64)));
    assert_eq!(label.chars().count(), 96);
}

#[tokio::test]
async fn sweep_isolates_a_failed_revoke_and_retries_its_durable_marker() {
    let (db, actor, key, _) = fixture("akl_sweep_partial_failure").await;
    let failing = start(&db).await;
    accept(&db, &actor, &key, &failing).await;
    let failing = expire_request(&db, &failing.user_code).await;
    let healthy = start(&db).await;
    accept(&db, &actor, &key, &healthy).await;
    let healthy = expire_request(&db, &healthy.user_code).await;
    let bad_id = failing.credential_id.as_deref().unwrap();
    let good_id = healthy.credential_id.as_deref().unwrap();
    db.run_command(doc! {"collMod": CREDENTIALS, "validator": {"$or": [{"_id": {"$ne": bad_id}}, {"is_active": true}]}, "validationLevel": "strict", "validationAction": "error"}).await.unwrap();
    assert_eq!(
        sweep_expired(&db).await.unwrap(),
        SweepResult {
            succeeded: 1,
            failed: 1
        }
    );
    let children = db.collection::<ApiKeyCredential>(CREDENTIALS);
    assert!(
        children
            .find_one(doc! {"_id": bad_id})
            .await
            .unwrap()
            .unwrap()
            .is_active
    );
    let revoked = children
        .find_one(doc! {"_id": good_id})
        .await
        .unwrap()
        .unwrap();
    assert!(!revoked.is_active);
    assert_eq!(
        revoked.revoked_reason,
        Some(CredentialRevokedReason::UndeliveredExpired)
    );
    let pending_cleanup = db
        .collection::<AgentKeyLoginRequest>(COLLECTION_NAME)
        .find_one(doc! {"_id": &failing.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending_cleanup.status, Status::Expired);
    assert_eq!(pending_cleanup.credential_id.as_deref(), Some(bad_id));
    assert!(pending_cleanup.delivery_credential_encrypted.is_none());
    let cleaned = db
        .collection::<AgentKeyLoginRequest>(COLLECTION_NAME)
        .find_one(doc! {"_id": &healthy.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cleaned.status, Status::Expired);
    assert!(cleaned.credential_id.is_none());
    db.run_command(doc! {"collMod": CREDENTIALS, "validator": {}})
        .await
        .unwrap();
    assert_eq!(sweep_expired(&db).await.unwrap().failed, 0);
    assert!(
        !children
            .find_one(doc! {"_id": bad_id})
            .await
            .unwrap()
            .unwrap()
            .is_active
    );
}

#[tokio::test]
async fn issuance_rejects_short_secret_without_inserting_a_child() {
    let (db, _, key, _) = fixture("akl_short_secret").await;
    let mut session = db.client().start_session().await.unwrap();
    for secret in ["", "nyxid_ag_1234567", "nyxid_ag_1234567\u{00e9}"] {
        assert!(matches!(
            credentials::issue(
                &db,
                &key,
                &Uuid::new_v4().to_string(),
                &Uuid::new_v4().to_string(),
                "CLI",
                None,
                secret,
                &mut session
            )
            .await,
            Err(AppError::ValidationError(_))
        ));
    }
    assert_eq!(
        db.collection::<ApiKeyCredential>(CREDENTIALS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[allow(clippy::too_many_arguments)]
async fn approve(
    db: &Database,
    encryption: &EncryptionKeys,
    hmac_key: &[u8],
    actor: &str,
    user_code: &str,
    selection: Selection,
    expires_at: Option<DateTime<Utc>>,
    ip: Option<&str>,
) -> AppResult<()> {
    super::approve(
        db, encryption, hmac_key, actor, user_code, selection, expires_at, ip, None,
    )
    .await
}

async fn poll(
    db: &Database,
    encryption: &EncryptionKeys,
    hmac_key: &[u8],
    device_code: &str,
) -> AppResult<Delivery> {
    super::poll(db, encryption, hmac_key, device_code, None, None).await
}

async fn deny(db: &Database, hmac_key: &[u8], actor: &str, user_code: &str) -> AppResult<()> {
    super::deny(db, hmac_key, actor, user_code, None, None).await
}

async fn fixture(name: &str) -> (Database, String, ApiKey, String) {
    let db = connect_transaction_test_database(name).await;
    crate::db::ensure_indexes(&db).await.expect("indexes");
    let actor = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&actor, UserType::Person))
        .await
        .unwrap();
    let secret = credentials::generate_secret().to_string();
    let key: ApiKey = bson::from_document(doc! {"_id": Uuid::new_v4().to_string(), "user_id": &actor, "name": "Shared agent", "key_prefix": &secret[..17], "key_hash": hash_token(&secret), "scopes": "read proxy", "is_active": true, "allow_all_services": false, "allow_all_nodes": false, "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(), "state_version": 1_i64}).unwrap();
    db.collection::<ApiKey>(API_KEYS)
        .insert_one(&key)
        .await
        .unwrap();
    (db, actor, key, secret)
}

async fn start(db: &Database) -> RequestOutput {
    request(
        db,
        HMAC_KEY,
        LoginClientContext {
            client_label: Some("workstation\n".into()),
            client_ip: Some("203.0.113.5".into()),
            ..Default::default()
        },
        Some("profile\n".into()),
    )
    .await
    .unwrap()
}

async fn accept(db: &Database, actor: &str, key: &ApiKey, request: &RequestOutput) {
    approve(
        db,
        &test_encryption_keys(),
        HMAC_KEY,
        actor,
        &request.user_code,
        Selection::Existing {
            permission_snapshot: None,
            api_key_id: key.id.clone(),
        },
        None,
        None,
    )
    .await
    .unwrap();
}

fn new_selection(overrides: serde_json::Value) -> Selection {
    let mut input = serde_json::json!({"kind": "new", "name": "New CLI", "scopes": "read proxy"});
    input
        .as_object_mut()
        .unwrap()
        .extend(overrides.as_object().unwrap().clone());
    serde_json::from_value(input).unwrap()
}

async fn expire_request(db: &Database, code: &str) -> AgentKeyLoginRequest {
    db.collection::<AgentKeyLoginRequest>(COLLECTION_NAME).find_one_and_update(
        doc! {"user_code_hmac": code_hash(HMAC_KEY, &normalized_code(code).unwrap())},
        doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}},
    ).return_document(ReturnDocument::After).await.unwrap().unwrap()
}

#[tokio::test]
async fn request_hmac_context_models_and_legacy_defaults() {
    let (db, _, _, _) = fixture("akl_models").await;
    let request = start(&db).await;
    assert!(request.device_code.starts_with("nyx_akl_"));
    assert_eq!(request.user_code.len(), 9);
    let row = find_by_user_code(&db, HMAC_KEY, &request.user_code)
        .await
        .unwrap();
    let doc = bson::to_document(&row).unwrap();
    assert!(doc.get_datetime("created_at").is_ok());
    assert!(doc.get_datetime("expires_at").is_ok());
    assert!(!format!("{doc:?}").contains(&request.device_code));
    assert!(!format!("{doc:?}").contains(&request.user_code));
    assert!(!format!("{row:?} {request:?}").contains(&row.device_code_hmac));
    assert_eq!(row.context.client_label.as_deref(), Some("workstation"));
    assert_eq!(row.context.requested_profile.as_deref(), Some("profile"));
    let mut legacy = doc;
    for field in [
        "requested_profile",
        "slow_down_increments",
        "key_was_created",
        "last_polled_at",
        "client_kind",
        "client_ip_attribution",
    ] {
        legacy.remove(field);
    }
    let restored: AgentKeyLoginRequest = bson::from_document(legacy).unwrap();
    assert_eq!(restored.slow_down_increments, 0);
    assert!(!restored.key_was_created);
    assert!(restored.context.requested_profile.is_none());
}

#[tokio::test]
async fn pending_slowdown_denial_and_replay_state_machine() {
    let (db, actor, key, _) = fixture("akl_states").await;
    let request = start(&db).await;
    let encryption = test_encryption_keys();
    assert!(matches!(
        poll(&db, &encryption, HMAC_KEY, "unknown").await,
        Err(AppError::AgentKeyLoginNotFound)
    ));
    assert!(matches!(
        poll(&db, &encryption, HMAC_KEY, &request.device_code).await,
        Err(AppError::AgentKeyLoginPending)
    ));
    assert!(matches!(
        poll(&db, &encryption, HMAC_KEY, &request.device_code).await,
        Err(AppError::AgentKeyLoginSlowDown)
    ));
    let row = find_by_user_code(&db, HMAC_KEY, &request.user_code)
        .await
        .unwrap();
    assert_eq!(row.slow_down_increments, 1);
    deny(&db, HMAC_KEY, &actor, &request.user_code)
        .await
        .unwrap();
    assert!(matches!(
        poll(&db, &encryption, HMAC_KEY, &request.device_code).await,
        Err(AppError::AgentKeyLoginDenied)
    ));
    assert!(matches!(
        approve(
            &db,
            &encryption,
            HMAC_KEY,
            &actor,
            &request.user_code,
            Selection::Existing {
                api_key_id: key.id,
                permission_snapshot: None
            },
            None,
            None
        )
        .await,
        Err(AppError::AgentKeyLoginDenied)
    ));
    assert_eq!(
        db.collection::<ApiKeyCredential>(CREDENTIALS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn existing_key_unchanged_and_concurrent_delivery_is_single_use() {
    let (db, actor, key, primary) = fixture("akl_delivery").await;
    let request = start(&db).await;
    accept(&db, &actor, &key, &request).await;
    let unchanged = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! {"_id": &key.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged, key);
    let encryption = test_encryption_keys();
    let (a, b) = tokio::join!(
        poll(&db, &encryption, HMAC_KEY, &request.device_code),
        poll(&db, &encryption, HMAC_KEY, &request.device_code)
    );
    let (delivery, loser) = match a {
        Ok(delivery) => (delivery, b),
        Err(error) => (b.unwrap(), Err(error)),
    };
    assert!(matches!(
        loser,
        Err(AppError::AgentKeyLoginAlreadyDelivered)
    ));
    assert_ne!(delivery.credential.as_str(), primary);
    let (owner, live, credential_id) = key_service::validate_api_key(&db, &delivery.credential)
        .await
        .unwrap();
    assert_eq!(owner, actor);
    assert_eq!(live.id, key.id);
    assert_eq!(
        credential_id.as_deref(),
        Some(delivery.credential_id.as_str())
    );
    assert!(!live.allow_all_services);
    assert_eq!(
        key_service::validate_api_key(&db, &primary)
            .await
            .unwrap()
            .2,
        None
    );
    let row = find_by_user_code(&db, HMAC_KEY, &request.user_code)
        .await
        .unwrap();
    assert_eq!(row.status, Status::Delivered);
    assert!(row.delivery_credential_encrypted.is_none());
    let child = db
        .collection::<ApiKeyCredential>(CREDENTIALS)
        .find_one(doc! {"_id": &delivery.credential_id})
        .await
        .unwrap()
        .unwrap();
    assert!(child.last_used_at.is_some());
    assert_eq!(child.secret_hash, hash_token(&delivery.credential));
    assert!(!format!("{child:?} {delivery:?}").contains(&child.secret_hash));
    assert!(!format!("{child:?} {delivery:?}").contains(delivery.credential.as_str()));
    let mut document = bson::to_document(&child).unwrap();
    assert!(document.get_datetime("last_used_at").is_ok());
    for field in [
        "kind",
        "expires_at",
        "revoked_at",
        "revoked_reason",
        "last_used_at",
    ] {
        document.remove(field);
    }
    let restored: ApiKeyCredential = bson::from_document(document).unwrap();
    assert_eq!(
        restored.kind,
        crate::models::api_key_credential::ApiKeyCredentialKind::AgentKeyLogin
    );
    assert!(restored.revoked_at.is_none());
}

#[tokio::test]
async fn new_key_defaults_and_validation_roll_back_the_entire_approval() {
    let (db, actor, _, _) = fixture("akl_new").await;
    let encryption = test_encryption_keys();
    for overrides in [
        serde_json::json!({"name": ""}),
        serde_json::json!({"scopes": "root"}),
        serde_json::json!({"allowed_service_ids": ["missing"]}),
        serde_json::json!({"allow_all_nodes": true, "allowed_node_ids": ["missing"]}),
        serde_json::json!({"scope_plan_digest": "stale"}),
        serde_json::json!({"platform": "unsupported"}),
    ] {
        let request = start(&db).await;
        assert!(
            approve(
                &db,
                &encryption,
                HMAC_KEY,
                &actor,
                &request.user_code,
                new_selection(overrides),
                None,
                None
            )
            .await
            .is_err()
        );
        assert_eq!(
            find_by_user_code(&db, HMAC_KEY, &request.user_code)
                .await
                .unwrap()
                .status,
            Status::Pending
        );
        assert_eq!(
            db.collection::<ApiKey>(API_KEYS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.collection::<ApiKeyCredential>(CREDENTIALS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    // Fail issuance after the new parent has been inserted into the transaction.
    db.run_command(doc! {"collMod": CREDENTIALS, "validator": {"$jsonSchema": {
        "bsonType": "object", "required": ["force_issuance_failure"]
    }}})
    .await
    .unwrap();
    let failed_request = start(&db).await;
    assert!(
        approve(
            &db,
            &encryption,
            HMAC_KEY,
            &actor,
            &failed_request.user_code,
            new_selection(serde_json::json!({})),
            None,
            None
        )
        .await
        .is_err()
    );
    assert_eq!(
        db.collection::<ApiKey>(API_KEYS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<ApiKeyCredential>(CREDENTIALS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        find_by_user_code(&db, HMAC_KEY, &failed_request.user_code)
            .await
            .unwrap()
            .status,
        Status::Pending
    );
    db.run_command(doc! {"collMod": CREDENTIALS, "validator": {}})
        .await
        .unwrap();
    let request = start(&db).await;
    approve(
        &db,
        &encryption,
        HMAC_KEY,
        &actor,
        &request.user_code,
        new_selection(serde_json::json!({})),
        None,
        None,
    )
    .await
    .unwrap();
    let delivered = poll(&db, &encryption, HMAC_KEY, &request.device_code)
        .await
        .unwrap();
    assert!(delivered.api_key.created_now);
    assert!(!delivered.api_key.allow_all_services);
    assert!(!delivered.api_key.allow_all_nodes);
    assert!(delivered.api_key.allowed_service_ids.is_empty());
    assert!(
        key_service::validate_api_key(&db, &delivered.credential)
            .await
            .is_ok()
    );
    credentials::web_revoke(&db, &actor, &delivered.api_key.id, &delivered.credential_id)
        .await
        .unwrap();
    assert!(
        key_service::validate_api_key(&db, &delivered.credential)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn approve_deny_race_commits_only_one_outcome() {
    let (db, actor, key, _) = fixture("akl_decision_race").await;
    let encryption = test_encryption_keys();
    for _ in 0..5 {
        let request = start(&db).await;
        let (approved, denied) = tokio::join!(
            approve(
                &db,
                &encryption,
                HMAC_KEY,
                &actor,
                &request.user_code,
                Selection::Existing {
                    permission_snapshot: None,
                    api_key_id: key.id.clone()
                },
                None,
                None
            ),
            deny(&db, HMAC_KEY, &actor, &request.user_code)
        );
        assert_ne!(approved.is_ok(), denied.is_ok());
        let row = find_by_user_code(&db, HMAC_KEY, &request.user_code)
            .await
            .unwrap();
        assert_eq!(
            row.status,
            if approved.is_ok() {
                Status::Approved
            } else {
                Status::Denied
            }
        );
        assert_eq!(row.credential_id.is_some(), approved.is_ok());
    }
}

#[tokio::test]
async fn poll_preview_and_sweep_expiry_revoke_undelivered_credentials() {
    let (db, actor, key, _) = fixture("akl_expiry").await;
    let encryption = test_encryption_keys();
    for path in ["poll", "preview", "sweep"] {
        let request = start(&db).await;
        accept(&db, &actor, &key, &request).await;
        let row = expire_request(&db, &request.user_code).await;
        let encrypted = row.delivery_credential_encrypted.as_ref().unwrap();
        let raw = encryption.decrypt(encrypted).await.unwrap();
        let secret = std::str::from_utf8(&raw).unwrap();
        match path {
            "poll" => assert!(matches!(
                poll(&db, &encryption, HMAC_KEY, &request.device_code).await,
                Err(AppError::AgentKeyLoginExpired)
            )),
            "preview" => assert_eq!(
                preview(
                    &db,
                    HMAC_KEY,
                    &request.user_code,
                    None,
                    AuthDeviceClientIpAttribution::Unavailable
                )
                .await
                .unwrap()
                .context
                .status,
                Status::Expired
            ),
            _ => {
                assert!(sweep_expired(&db).await.unwrap().succeeded > 0);
            }
        }
        assert!(key_service::validate_api_key(&db, secret).await.is_err());
        let child = db
            .collection::<ApiKeyCredential>(CREDENTIALS)
            .find_one(doc! {"_id": row.credential_id.unwrap()})
            .await
            .unwrap()
            .unwrap();
        assert!(!child.is_active);
        assert_eq!(
            child.revoked_reason,
            Some(CredentialRevokedReason::UndeliveredExpired)
        );
    }
    let request = start(&db).await;
    approve(
        &db,
        &encryption,
        HMAC_KEY,
        &actor,
        &request.user_code,
        new_selection(serde_json::json!({})),
        None,
        None,
    )
    .await
    .unwrap();
    let row = expire_request(&db, &request.user_code).await;
    sweep_expired(&db).await.unwrap();
    assert!(
        db.collection::<ApiKey>(API_KEYS)
            .find_one(doc! {"_id": row.api_key_id.unwrap()})
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn failed_delivery_preparation_remains_sweepable() {
    let (db, actor, key, _) = fixture("akl_delivery_failure").await;
    let request = start(&db).await;
    accept(&db, &actor, &key, &request).await;
    let row = find_by_user_code(&db, HMAC_KEY, &request.user_code)
        .await
        .unwrap();
    let encryption = test_encryption_keys();
    let secret = encryption
        .decrypt(row.delivery_credential_encrypted.as_deref().unwrap())
        .await
        .unwrap();
    db.collection::<AgentKeyLoginRequest>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"delivery_credential_encrypted": bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic, bytes: vec![0; 3],
            }}},
        )
        .await
        .unwrap();
    assert!(
        poll(&db, &encryption, HMAC_KEY, &request.device_code)
            .await
            .is_err()
    );
    assert_eq!(
        find_by_user_code(&db, HMAC_KEY, &request.user_code)
            .await
            .unwrap()
            .status,
        Status::Approved
    );
    expire_request(&db, &request.user_code).await;
    sweep_expired(&db).await.unwrap();
    assert!(
        key_service::validate_api_key(&db, std::str::from_utf8(&secret).unwrap())
            .await
            .is_err()
    );
    let child = db
        .collection::<ApiKeyCredential>(CREDENTIALS)
        .find_one(doc! {"_id": row.credential_id.unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        child.revoked_reason,
        Some(CredentialRevokedReason::UndeliveredExpired)
    );
}

#[tokio::test]
async fn rolling_cleanup_markers_preserve_new_parents_and_reserve_old_pending_parents() {
    let (db, actor, _, _) = fixture("akl_rolling_cleanup").await;
    let encryption = test_encryption_keys();
    let request = start(&db).await;
    approve(
        &db,
        &encryption,
        HMAC_KEY,
        &actor,
        &request.user_code,
        new_selection(serde_json::json!({})),
        None,
        None,
    )
    .await
    .unwrap();
    let rows = db.collection::<AgentKeyLoginRequest>(COLLECTION_NAME);
    let row = rows
        .find_one(doc! {"device_code_hmac":code_hash(HMAC_KEY, &request.device_code)})
        .await
        .unwrap()
        .unwrap();
    assert!(
        !row.key_was_created,
        "an old sweeper must not consider this parent deletable"
    );
    assert!(row.key_created_by_approval);
    let parent = row.api_key_id.as_deref().unwrap();
    assert!(eligible_key(&db, &actor, parent).await.is_ok());
    assert_eq!(
        rows.count_documents(doc! {"_id":&row.id,"key_was_created":true})
            .await
            .unwrap(),
        0
    );

    // Emulate an approval written by the preceding release. New consumers
    // cannot reuse its parent while that release can still delete it.
    rows.update_one(
        doc! {"_id":&row.id},
        doc! {"$set":{"key_was_created":true,"key_created_by_approval":false}},
    )
    .await
    .unwrap();
    assert!(matches!(
        eligible_key(&db, &actor, parent).await,
        Err(AppError::AgentKeyLoginKeyIneligible)
    ));
    let delivery = poll(&db, &encryption, HMAC_KEY, &request.device_code)
        .await
        .unwrap();
    assert!(
        key_service::validate_api_key(&db, &delivery.credential)
            .await
            .is_ok()
    );
    assert!(eligible_key(&db, &actor, parent).await.is_ok());
}

#[tokio::test]
async fn abandoned_new_parent_survives_reuse_and_concurrent_issuance() {
    let (db, actor, _, _) = fixture("akl_reused_parent_cleanup").await;
    let encryption = test_encryption_keys();
    for concurrent in [false, true] {
        let first = start(&db).await;
        approve(
            &db,
            &encryption,
            HMAC_KEY,
            &actor,
            &first.user_code,
            new_selection(serde_json::json!({})),
            None,
            None,
        )
        .await
        .unwrap();
        let row = expire_request(&db, &first.user_code).await;
        let second = start(&db).await;
        let approval = approve(
            &db,
            &encryption,
            HMAC_KEY,
            &actor,
            &second.user_code,
            Selection::Existing {
                permission_snapshot: None,
                api_key_id: row.api_key_id.clone().unwrap(),
            },
            None,
            None,
        );
        let approved = if concurrent {
            let (approved, swept) = tokio::join!(approval, sweep_expired(&db));
            assert_eq!(swept.unwrap().failed, 0);
            approved
        } else {
            let approved = approval.await;
            assert_eq!(sweep_expired(&db).await.unwrap().failed, 0);
            approved
        };
        if !concurrent {
            assert!(approved.is_ok());
        }
        if approved.is_ok() {
            let delivery = poll(&db, &encryption, HMAC_KEY, &second.device_code)
                .await
                .unwrap();
            assert!(
                key_service::validate_api_key(&db, &delivery.credential)
                    .await
                    .is_ok()
            );
        } else {
            assert_eq!(
                db.collection::<ApiKeyCredential>(CREDENTIALS)
                    .count_documents(doc! {"api_key_id": &row.api_key_id, "is_active": true})
                    .await
                    .unwrap(),
                0
            );
        }
        let child = db
            .collection::<ApiKeyCredential>(CREDENTIALS)
            .find_one(doc! {"_id": row.credential_id})
            .await
            .unwrap()
            .unwrap();
        assert!(!child.is_active);
    }
}

#[tokio::test]
async fn expiry_and_late_preview_do_not_revoke_delivered_credentials() {
    let (db, actor, key, _) = fixture("akl_delivered_expiry").await;
    let request = start(&db).await;
    accept(&db, &actor, &key, &request).await;
    let delivery = poll(&db, &test_encryption_keys(), HMAC_KEY, &request.device_code)
        .await
        .unwrap();
    expire_request(&db, &request.user_code).await;
    sweep_expired(&db).await.unwrap();
    let result = preview(
        &db,
        HMAC_KEY,
        &request.user_code,
        None,
        AuthDeviceClientIpAttribution::Unavailable,
    )
    .await
    .unwrap();
    assert_eq!(result.context.status, Status::Delivered);
    assert!(
        key_service::validate_api_key(&db, &delivery.credential)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn live_parent_authority_expiry_logout_rotation_and_revocation() {
    let (db, actor, key, _) = fixture("akl_lifecycle").await;
    for reason in [
        "logout",
        "rotation",
        "revocation",
        "child_expiry",
        "parent_expiry",
        "scope",
    ] {
        db.collection::<ApiKey>(API_KEYS)
            .update_one(
                doc! {"_id": &key.id},
                doc! {"$set": {"is_active": true, "expires_at": bson::Bson::Null}},
            )
            .await
            .unwrap();
        let request = start(&db).await;
        accept(&db, &actor, &key, &request).await;
        let delivery = poll(&db, &test_encryption_keys(), HMAC_KEY, &request.device_code)
            .await
            .unwrap();
        match reason {
            "logout" => credentials::revoke(
                &db,
                &delivery.credential_id,
                CredentialRevokedReason::Logout,
            )
            .await
            .unwrap(),
            "rotation" => {
                key_service::rotate_api_key(&db, &actor, &key.id)
                    .await
                    .unwrap();
            }
            "revocation" => key_service::delete_api_key(&db, &actor, &key.id)
                .await
                .unwrap(),
            "child_expiry" => {
                db.collection::<ApiKeyCredential>(CREDENTIALS).update_one(doc! {"_id": &delivery.credential_id}, doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}}).await.unwrap();
            }
            "parent_expiry" => {
                db.collection::<ApiKey>(API_KEYS).update_one(doc! {"_id": &key.id}, doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}}).await.unwrap();
            }
            _ => {
                db.collection::<ApiKey>(API_KEYS).update_one(doc! {"_id": &key.id}, doc! {"$set": {"scopes": "read", "rate_limit_per_second": 2, "allowed_service_ids": ["changed"]}}).await.unwrap();
                let (_, live, _) = key_service::validate_api_key(&db, &delivery.credential)
                    .await
                    .unwrap();
                assert_eq!(live.scopes, "read");
                assert_eq!(live.rate_limit_per_second, Some(2));
                assert_eq!(live.allowed_service_ids, vec!["changed"]);
                continue;
            }
        }
        assert!(
            key_service::validate_api_key(&db, &delivery.credential)
                .await
                .is_err(),
            "{reason}"
        );
        if matches!(reason, "rotation" | "revocation") {
            let child = db
                .collection::<ApiKeyCredential>(CREDENTIALS)
                .find_one(doc! {"_id": &delivery.credential_id})
                .await
                .unwrap()
                .unwrap();
            assert!(!child.is_active);
            assert_eq!(
                child.revoked_reason,
                Some(if reason == "rotation" {
                    CredentialRevokedReason::ParentRotated
                } else {
                    CredentialRevokedReason::ParentRevoked
                })
            );
        }
    }
}

#[tokio::test]
async fn eligibility_org_admin_and_management_ownership() {
    let (db, actor, key, _) = fixture("akl_eligibility").await;
    let outsider = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&outsider, UserType::Person))
        .await
        .unwrap();
    assert!(matches!(
        eligible_key(&db, &outsider, &key.id).await,
        Err(AppError::AgentKeyLoginKeyIneligible)
    ));
    assert!(credentials::list(&db, &outsider, &key.id).await.is_err());
    let org = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    db.collection::<ApiKey>(API_KEYS)
        .update_one(doc! {"_id": &key.id}, doc! {"$set": {"user_id": &org}})
        .await
        .unwrap();
    db.collection::<crate::models::org_membership::OrgMembership>(MEMBERSHIPS)
        .insert_one(test_membership(&org, &actor, OrgRole::Member, None))
        .await
        .unwrap();
    assert!(eligible_key(&db, &actor, &key.id).await.is_err());
    db.collection::<bson::Document>(MEMBERSHIPS)
        .update_one(
            doc! {"org_user_id": &org, "member_user_id": &actor},
            doc! {"$set": {"role": "admin"}},
        )
        .await
        .unwrap();
    assert!(eligible_key(&db, &actor, &key.id).await.is_ok());
    let request = start(&db).await;
    let choices = options(&db, HMAC_KEY, &actor, &request.user_code)
        .await
        .unwrap();
    assert_eq!(choices.keys.len(), 1);
    assert_eq!(choices.keys[0].owner_type, "org");
    assert_eq!(choices.orgs.len(), 1);
    for update in [
        doc! {"purpose": "scheduled_invocation"},
        doc! {"purpose": "general", "is_active": false},
        doc! {"is_active": true, "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))},
    ] {
        db.collection::<ApiKey>(API_KEYS)
            .update_one(doc! {"_id": &key.id}, doc! {"$set": update})
            .await
            .unwrap();
        assert!(matches!(
            eligible_key(&db, &actor, &key.id).await,
            Err(AppError::AgentKeyLoginKeyIneligible)
        ));
        assert!(
            options(&db, HMAC_KEY, &actor, &request.user_code)
                .await
                .unwrap()
                .keys
                .is_empty()
        );
    }
}

#[test]
fn credential_expiry_cannot_outlive_parent() {
    let parent = Utc::now() + Duration::days(1);
    assert_eq!(
        credentials::credential_expiry(Some(parent), None).unwrap(),
        Some(parent)
    );
    assert!(
        credentials::credential_expiry(Some(parent), Some(parent + Duration::seconds(1))).is_err()
    );
    assert!(credentials::credential_expiry(None, Some(Utc::now() - Duration::seconds(1))).is_err());
    assert_eq!(credentials::credential_expiry(None, None).unwrap(), None);
}

#[tokio::test]
async fn auto_connected_login_options_provision_and_mark_platform_services() {
    let (db, actor, _, _) = fixture("auto_connected_login_options").await;
    let catalog = crate::test_utils::test_auto_connected_catalog_service();
    db.collection::<crate::models::downstream_service::DownstreamService>(
        crate::models::downstream_service::COLLECTION_NAME,
    )
    .insert_one(&catalog)
    .await
    .unwrap();
    let options = options_for_actor(&db, &actor).await.unwrap();
    assert_eq!(options.personal_owner_id, actor);
    let service = options
        .services
        .iter()
        .find(|service| service.name == catalog.slug)
        .unwrap();
    assert!(service.auto_connected);
    assert_eq!(service.owner_id, actor);
    assert_eq!(
        serde_json::to_value(service).unwrap()["auto_connected"],
        true
    );
    let again = options_for_actor(&db, &actor).await.unwrap();
    assert_eq!(
        again
            .services
            .iter()
            .filter(|row| row.id == service.id)
            .count(),
        1
    );
    db.drop().await.unwrap();
}

async fn consent_fixture(name: &str) -> (Database, String, ApiKey, String, String) {
    use crate::models::user_api_key::{COLLECTION_NAME as CONNECTIONS, UserApiKey};
    let (db, actor, key, _) = fixture(name).await;
    let credential = Uuid::new_v4().to_string();
    let service_id = Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    let external: UserApiKey = bson::from_document(doc! {
        "_id": &credential, "user_id": &actor, "label": "Read connection", "credential_type": "oauth2",
        "status": "active", "credential_epoch": 1_i64, "token_scopes": "repo:read",
        "access_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![1,2,3] },
        "refresh_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![4,5,6] },
        "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::hours(1)),
        "created_at": now, "updated_at": now,
    }).unwrap();
    db.collection::<UserApiKey>(CONNECTIONS)
        .insert_one(external)
        .await
        .unwrap();
    let mut service = crate::test_utils::test_user_service(
        &service_id,
        &actor,
        "github-personal",
        "endpoint",
        None,
        None,
    );
    service.api_key_id = Some(credential.clone());
    service.auth_method = "bearer".into();
    db.collection::<UserService>(SERVICES)
        .insert_one(service)
        .await
        .unwrap();
    mutations::update_one(
        &db,
        doc! {"_id": &key.id},
        doc! {"$set": {"allowed_service_ids": [&service_id]}},
        None,
    )
    .await
    .unwrap();
    let key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! {"_id": &key.id})
        .await
        .unwrap()
        .unwrap();
    (db, actor, key, service_id, credential)
}

#[tokio::test]
async fn permission_summary_resolves_agent_override_and_never_falls_back_when_missing_or_inactive()
{
    use crate::models::user_api_key::{COLLECTION_NAME as CONNECTIONS, UserApiKey};
    let (db, actor, key, service, credential) = consent_fixture("login_bound_permissions").await;
    let mut override_key = db
        .collection::<UserApiKey>(CONNECTIONS)
        .find_one(doc! {"_id": &credential})
        .await
        .unwrap()
        .unwrap();
    override_key.id = Uuid::new_v4().to_string();
    override_key.label = "Write connection".into();
    override_key.token_scopes = Some("repo:read repo:write email".into());
    db.collection::<UserApiKey>(CONNECTIONS)
        .insert_one(&override_key)
        .await
        .unwrap();
    crate::services::agent_binding_service::create_binding(
        &db,
        &actor,
        &key.id,
        &service,
        &override_key.id,
    )
    .await
    .unwrap();
    let summary = key_summary(&db, &key, false).await.unwrap();
    assert_eq!(summary.effective_services[0].label, "Write connection");
    assert_eq!(
        summary.effective_services[0]
            .granted_scopes
            .as_ref()
            .unwrap(),
        &["email", "repo:read", "repo:write"]
    );
    db.collection::<UserApiKey>(CONNECTIONS)
        .update_one(
            doc! {"_id": &override_key.id},
            doc! {"$set": {"status": "revoked"}},
        )
        .await
        .unwrap();
    assert_eq!(
        key_summary(&db, &key, false)
            .await
            .unwrap()
            .effective_services[0]
            .status,
        "revoked"
    );
    db.collection::<UserApiKey>(CONNECTIONS)
        .delete_one(doc! {"_id": &override_key.id})
        .await
        .unwrap();
    let missing = key_summary(&db, &key, false).await.unwrap();
    assert!(missing.effective_services[0].credential_missing);
    assert!(missing.effective_services[0].granted_scopes.is_none());
}

#[tokio::test]
async fn refreshable_oauth_remains_eligible_and_refresh_does_not_change_permission_snapshot() {
    let (db, _, key, _, credential) = consent_fixture("login_refresh_stable_consent").await;
    let before = key_summary(&db, &key, false).await.unwrap();
    assert_eq!(before.effective_services[0].status, "active");
    assert_eq!(
        before.effective_services[0].connection_status.as_deref(),
        Some("active")
    );
    assert!(!before.effective_services[0].credential_missing);
    assert!(before.effective_services[0].expires_at.is_none());
    db.collection::<bson::Document>("user_api_keys").update_one(doc! {"_id": &credential}, doc! {"$set": {
        "expires_at": bson::DateTime::from_chrono(Utc::now() + Duration::hours(1)),
        "updated_at": bson::DateTime::now(),
        "access_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![9,8,7] }
    }}).await.unwrap();
    let after = key_summary(&db, &key, false).await.unwrap();
    assert_eq!(before.permission_snapshot, after.permission_snapshot);
}

#[tokio::test]
async fn new_client_consent_rejects_broadened_existing_and_new_connection_grants_atomically() {
    for existing in [true, false] {
        let (db, actor, key, service, credential) = consent_fixture("login_consent_drift").await;
        let options = options_for_actor(&db, &actor).await.unwrap();
        let selection = if existing {
            Selection::Existing {
                api_key_id: key.id.clone(),
                permission_snapshot: Some(options.keys[0].permission_snapshot.clone()),
            }
        } else {
            serde_json::from_value(serde_json::json!({"kind":"new", "name":"Reviewed reader", "scopes":"read proxy", "allowed_service_ids":[service], "connection_snapshots":[{"service_id":service,"permission_snapshot":options.connections[0].permission_snapshot}]})).unwrap()
        };
        let request = start(&db).await;
        db.collection::<bson::Document>("user_api_keys")
            .update_one(
                doc! {"_id": &credential},
                doc! {"$set": {"token_scopes": "repo:read repo:write", "credential_epoch": 2_i64}},
            )
            .await
            .unwrap();
        let result = approve(
            &db,
            &test_encryption_keys(),
            HMAC_KEY,
            &actor,
            &request.user_code,
            selection,
            None,
            None,
        )
        .await;
        assert!(matches!(result, Err(AppError::Conflict(_))), "{result:?}");
        assert_eq!(
            db.collection::<ApiKeyCredential>(CREDENTIALS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<ApiKey>(API_KEYS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            find_by_user_code(&db, HMAC_KEY, &request.user_code)
                .await
                .unwrap()
                .status,
            Status::Pending
        );
    }
}

#[tokio::test]
async fn new_client_unchanged_consent_issues_real_existing_and_new_credentials() {
    for existing in [true, false] {
        let (db, actor, key, service, _) = consent_fixture("login_consent_approve").await;
        let options = options_for_actor(&db, &actor).await.unwrap();
        let selection = if existing {
            Selection::Existing {
                api_key_id: key.id.clone(),
                permission_snapshot: Some(options.keys[0].permission_snapshot.clone()),
            }
        } else {
            serde_json::from_value(serde_json::json!({"kind":"new", "name":"Reviewed reader", "scopes":"read proxy", "allowed_service_ids":[service], "connection_snapshots":[{"service_id":service,"permission_snapshot":options.connections[0].permission_snapshot}]})).unwrap()
        };
        let request = start(&db).await;
        approve(
            &db,
            &test_encryption_keys(),
            HMAC_KEY,
            &actor,
            &request.user_code,
            selection,
            None,
            None,
        )
        .await
        .unwrap();
        let delivery = poll(&db, &test_encryption_keys(), HMAC_KEY, &request.device_code)
            .await
            .unwrap();
        assert!(delivery.credential.starts_with("nyxid_ag_"));
        assert_eq!(delivery.api_key.allowed_service_ids, [service]);
    }
}

#[tokio::test]
async fn legacy_provider_fallback_scopes_are_unreported_even_when_cached_scopes_exist() {
    let (db, _, key, _, credential) = consent_fixture("login_legacy_provider_unknown").await;
    db.collection::<bson::Document>("user_api_keys").update_one(doc! {"_id": &credential}, doc! {"$set": {"provider_config_id": "legacy-provider", "connection_id": bson::Bson::Null}}).await.unwrap();
    let summary = key_summary(&db, &key, false).await.unwrap();
    assert!(summary.effective_services[0].granted_scopes.is_none());
    assert!(!summary.effective_services[0].credential_missing);
}

#[tokio::test]
async fn unchanged_unavailable_extras_do_not_prevent_existing_key_issuance() {
    let (db, actor, key, service, _) = consent_fixture("login_unavailable_extras").await;
    let inactive_id = Uuid::new_v4().to_string();
    let missing_id = Uuid::new_v4().to_string();
    let mut inactive = crate::test_utils::test_user_service(
        &inactive_id,
        &actor,
        "disabled-extra",
        "endpoint",
        None,
        None,
    );
    inactive.is_active = false;
    db.collection::<UserService>(SERVICES)
        .insert_one(inactive)
        .await
        .unwrap();
    mutations::update_one(
        &db,
        doc! {"_id": &key.id},
        doc! {"$set": {"allowed_service_ids": [&service, &inactive_id, &missing_id]}},
        None,
    )
    .await
    .unwrap();
    let options = options_for_actor(&db, &actor).await.unwrap();
    assert_eq!(options.keys[0].effective_services.len(), 2);
    let request = start(&db).await;
    approve(
        &db,
        &test_encryption_keys(),
        HMAC_KEY,
        &actor,
        &request.user_code,
        Selection::Existing {
            api_key_id: key.id,
            permission_snapshot: Some(options.keys[0].permission_snapshot.clone()),
        },
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        db.collection::<ApiKeyCredential>(CREDENTIALS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn concurrent_post_read_scope_change_conflicts_then_rejects_stale_consent() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let (db, actor, key, _, credential) = consent_fixture("login_concurrent_consent").await;
    let summary = key_summary(&db, &key, false).await.unwrap();
    let observed = Arc::new(tokio::sync::Notify::new());
    let changed = Arc::new(tokio::sync::Notify::new());
    let attempts = Arc::new(AtomicUsize::new(0));
    let approve_db = db.clone();
    let observed_approval = observed.clone();
    let changed_approval = changed.clone();
    let tries = attempts.clone();
    let approval = async move {
        let mut session = approve_db.client().start_session().await.unwrap();
        session
            .start_transaction()
            .and_run2(async move |session| {
                let attempt = tries.fetch_add(1, Ordering::SeqCst);
                let result: AppResult<()> = async {
                    // Pin the issuance transaction's snapshot before the concurrent
                    // provider mutation. The real flow pins it when claiming the request.
                    approve_db
                        .collection::<ApiKey>(API_KEYS)
                        .find_one(doc! {"_id": &key.id})
                        .session(&mut *session)
                        .await?;
                    if attempt == 0 {
                        observed_approval.notify_one();
                        changed_approval.notified().await;
                    }
                    issue_selected(
                        &approve_db,
                        &actor,
                        "concurrent-review",
                        &LoginClientContext::default(),
                        None,
                        &Selection::Existing {
                            api_key_id: key.id.clone(),
                            permission_snapshot: Some(summary.permission_snapshot.clone()),
                        },
                        None,
                        &key.id,
                        "concurrent-child",
                        &credentials::generate_secret(),
                        &mut *session,
                    )
                    .await?;
                    Ok(())
                }
                .await;
                mutations::transaction_result(result)
            })
            .await
            .map_err(mutations::map_transaction_error)
    };
    let mutation = async {
        observed.notified().await;
        db.collection::<bson::Document>("user_api_keys")
            .update_one(
                doc! {"_id": &credential},
                doc! {"$set": {"token_scopes": "repo:read repo:write", "credential_epoch": 2_i64}},
            )
            .await
            .unwrap();
        changed.notify_one();
    };
    let (result, ()) = tokio::join!(approval, mutation);
    assert!(matches!(result, Err(AppError::Conflict(_))), "{result:?}");
    assert!(
        attempts.load(Ordering::SeqCst) >= 2,
        "the post-read mutation must force a transaction retry"
    );
    assert_eq!(
        db.collection::<ApiKeyCredential>(CREDENTIALS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn consent_platform_grants_are_owner_bound_and_use_live_platform_readiness() {
    use crate::models::downstream_service::{
        COLLECTION_NAME as CATALOG, DownstreamService, PlatformKeyAudience, PlatformKeyConfig,
    };
    let (db, actor, mut key, _, _) = consent_fixture("login_platform_consent").await;
    let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
    catalog.id = Uuid::new_v4().to_string();
    catalog.slug = "consent-platform".into();
    catalog.is_active = true;
    catalog.service_type = "http".into();
    catalog.auth_method = "bearer".into();
    catalog.auth_key_name = "Authorization".into();
    catalog.provider_config_id = None;
    catalog.credential_encrypted = vec![1]; // Intentionally not decryptable: consent reads metadata only.
    catalog.platform_key = Some(PlatformKeyConfig {
        enabled: true,
        audience: PlatformKeyAudience::Public,
        allowed_owner_ids: vec![],
    });
    db.collection::<DownstreamService>(CATALOG)
        .insert_one(&catalog)
        .await
        .unwrap();
    let mut expected = Vec::new();
    for (owner, explicit_binding, active) in [
        (actor.as_str(), false, true),
        (actor.as_str(), true, true),
        (actor.as_str(), true, false),
        ("foreign-owner", true, true),
    ] {
        let id = Uuid::new_v4().to_string();
        let mut service =
            crate::test_utils::test_user_service(&id, owner, &id, "endpoint", None, None);
        service.catalog_service_id = Some(catalog.id.clone());
        service.api_key_id = None;
        service.auth_method = "bearer".into();
        service.auth_key_name = "Authorization".into();
        service.is_active = active;
        if explicit_binding {
            service.credential_binding = Some("platform".into());
        } else {
            service.source = Some(crate::models::user_service::AUTO_PROVISION_SOURCE.into());
        }
        db.collection::<UserService>(SERVICES)
            .insert_one(service)
            .await
            .unwrap();
        if owner == actor.as_str() && active {
            expected.push(id);
        }
    }
    key.allow_auto_connected_services = true;
    let enabled = key_summary(&db, &key, false).await.unwrap();
    assert_eq!(enabled.effective_services.len(), 3);
    for id in &expected {
        assert!(
            enabled
                .allowed_services
                .iter()
                .find(|s| &s.id == id)
                .unwrap()
                .auto_connected
        );
        let service = enabled
            .effective_services
            .iter()
            .find(|s| &s.id == id)
            .unwrap();
        assert!(service.auto_connected && service.is_active && !service.credential_missing);
        assert_eq!(service.owner_id, actor);
        assert_eq!(service.status, "active");
        assert_eq!(service.credential_binding, "platform");
        assert!(service.granted_scopes.is_none());
    }
    key.allow_auto_connected_services = false;
    assert_ne!(
        enabled.permission_snapshot,
        key_summary(&db, &key, false)
            .await
            .unwrap()
            .permission_snapshot
    );
    key.allow_auto_connected_services = true;
    db.collection::<bson::Document>(CATALOG)
        .update_one(
            doc! {"_id": &catalog.id},
            doc! {"$set": {"platform_key.enabled": false}},
        )
        .await
        .unwrap();
    let disabled = key_summary(&db, &key, false).await.unwrap();
    for service in disabled
        .effective_services
        .iter()
        .filter(|s| expected.contains(&s.id))
    {
        assert!(service.credential_missing);
        assert_eq!(service.status, "missing");
    }
    assert_ne!(enabled.permission_snapshot, disabled.permission_snapshot);
    db.drop().await.unwrap();
}
