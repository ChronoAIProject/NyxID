use super::*;
use crate::crypto::token::hash_token;
use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgRole};
use crate::models::user::UserType;
use crate::test_utils::{
    connect_transaction_test_database, test_encryption_keys, test_membership, test_user,
};

const HMAC_KEY: &[u8] = b"agent-key-login-test-domain-key";

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
    assert_eq!(row.requested_profile.as_deref(), Some("profile"));
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
    assert!(restored.requested_profile.is_none());
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
            Selection::Existing { api_key_id: key.id },
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
                assert!(sweep_expired(&db).await.unwrap() > 0);
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
            .is_none()
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
