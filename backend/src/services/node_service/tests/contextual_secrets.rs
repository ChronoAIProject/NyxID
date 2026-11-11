use super::*;
use crate::crypto::{
    key_provider::{KeyProvider, WrappedKey},
    local_key_provider::LocalKeyProvider,
};
use std::sync::Arc;

#[tokio::test]
async fn node_adapter_authenticates_row_owner_field_and_explicit_format() {
    let mut keys = test_encryption_keys();
    let mut node = make_node("owner", "contextual-node");
    let secret = "ab".repeat(32);
    node.signing_secret_hash = hash_token(&secret);
    node.signing_secret_encrypted = Some(
        encrypt_signing_secret(&keys, &node.id, &node.user_id, secret.as_bytes())
            .await
            .unwrap(),
    );
    node.signing_secret_context_version = Some(3);
    assert_eq!(
        *signing_secret_from_node(&keys, &node).await.unwrap(),
        vec![0xab; 32]
    );
    let mut swapped = node.clone();
    swapped.id = Uuid::new_v4().to_string();
    assert!(signing_secret_from_node(&keys, &swapped).await.is_err());
    swapped = node.clone();
    swapped.user_id = "successor-owner".into();
    assert!(signing_secret_from_node(&keys, &swapped).await.is_err());
    swapped = node.clone();
    let blob = swapped.signing_secret_encrypted.as_mut().unwrap();
    let last = blob.len() - 1;
    blob[last] ^= 1;
    assert!(signing_secret_from_node(&keys, &swapped).await.is_err());
    // A mutable version flag cannot authorize a downgrade after trusted cutover.
    keys.set_allow_legacy_record_reads(false);
    swapped.signing_secret_context_version = None;
    swapped.signing_secret_encrypted = Some(keys.encrypt(secret.as_bytes()).await.unwrap());
    assert!(signing_secret_from_node(&keys, &swapped).await.is_err());
    assert_eq!(
        *signing_secret_from_node(&keys, &node).await.unwrap(),
        vec![0xab; 32]
    );
    swapped.signing_secret_context_version = Some(99);
    assert!(signing_secret_from_node(&keys, &swapped).await.is_err());
}

#[tokio::test]
async fn authenticated_malformed_or_hash_mismatched_secrets_never_become_ready() {
    let db =
        crate::test_utils::connect_transaction_test_database("node_secret_semantic_validation")
            .await;
    let keys = test_encryption_keys();
    for contextual in [false, true] {
        for material in [
            "".to_owned(),
            "ab".repeat(16),
            "not-hex".to_owned(),
            "cd".repeat(32),
        ] {
            let mut node = make_node("owner", "invalid-material");
            // Last case is a valid length with intentionally mismatched hash.
            node.signing_secret_hash = hash_token(&"ab".repeat(32));
            node.signing_secret_context_version = contextual.then_some(3);
            node.signing_secret_encrypted = Some(if contextual {
                encrypt_signing_secret(&keys, &node.id, &node.user_id, material.as_bytes())
                    .await
                    .unwrap()
            } else {
                keys.encrypt(material.as_bytes()).await.unwrap()
            });
            db.collection::<Node>(NODES)
                .insert_one(&node)
                .await
                .unwrap();
            let original = db
                .collection::<bson::Document>(NODES)
                .find_one(doc! {"_id":&node.id})
                .await
                .unwrap()
                .unwrap();
            assert!(signing_secret_from_node(&keys, &node).await.is_err());
            assert!(verify_current_signing_secret(&keys, &node).await.is_err());
            assert!(
                migrate_signing_secret_snapshot(&db, &keys, &bson::to_document(&node).unwrap())
                    .await
                    .is_err()
            );
            assert_eq!(
                db.collection::<bson::Document>(NODES)
                    .find_one(doc! {"_id":&node.id})
                    .await
                    .unwrap()
                    .unwrap(),
                original
            );
        }
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_v0_magic_collision_uses_only_explicit_legacy_contract() {
    use aes_gcm::{
        Aes256Gcm, Nonce,
        aead::{Aead, KeyInit},
    };
    let keys = test_encryption_keys();
    let mut node = make_node("legacy-owner", "legacy-node");
    // An actual authenticated v0 fixture whose RANDOM nonce could equal v3 magic.
    let nonce = *b"\x03NYX12345678";
    let secret = "cd".repeat(32);
    node.signing_secret_hash = hash_token(&secret);
    let mut blob = nonce.to_vec();
    blob.extend(
        Aes256Gcm::new_from_slice(&[0x11; 32])
            .unwrap()
            .encrypt(Nonce::from_slice(&nonce), secret.as_bytes())
            .unwrap(),
    );
    node.signing_secret_encrypted = Some(blob);
    assert_eq!(
        *signing_secret_from_node(&keys, &node).await.unwrap(),
        vec![0xcd; 32]
    );
    node.signing_secret_context_version = Some(3);
    assert!(
        signing_secret_from_node(&keys, &node).await.is_err(),
        "new-format failure must not retry v0"
    );
}

pub(super) async fn transfer_fixture(db: &mongodb::Database) -> (String, String) {
    let actor = Uuid::new_v4().to_string();
    let org = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_many([
            test_user(&actor, UserType::Person),
            test_user(&org, UserType::Org),
        ])
        .await
        .unwrap();
    db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
        .insert_one(test_membership(&org, &actor, OrgRole::Admin, None))
        .await
        .unwrap();
    (actor, org)
}

#[tokio::test]
async fn live_registration_capability_checks_role_only_at_issuance() {
    let Some(db) = connect_test_database("node_registration_capability_contract").await else {
        return;
    };
    let (actor, org) = transfer_fixture(&db).await;
    let (_, raw, _) =
        create_registration_token_for_actor(&db, &actor, &org, "issued-capability", 10, 300)
            .await
            .unwrap();
    // Seed the capability contract independently of the revocation service.
    // Production demotion atomically removes the pending token (covered below).
    // Redemption must not reinterpret an extant issued capability by role.
    db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
        .update_one(
            doc! {"org_user_id":&org,"member_user_id":&actor},
            doc! {"$set":{"role":"member"}},
        )
        .await
        .unwrap();
    let (node, _, _) = register_node(&db, &test_encryption_keys(), &raw, None)
        .await
        .unwrap();
    assert_eq!(node.user_id, org);
    assert!(
        create_registration_token_for_actor(&db, &actor, &org, "new-capability", 10, 300)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn admin_departure_retires_issuer_and_legacy_pending_tokens_without_deleting_org_nodes() {
    let Some(db) = connect_test_database("node_registration_issuer_departure").await else {
        return;
    };
    let (actor, org) = transfer_fixture(&db).await;
    let successor = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&successor, UserType::Person))
        .await
        .unwrap();
    let membership = test_membership(&org, &successor, OrgRole::Admin, None);
    db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
        .insert_one(&membership)
        .await
        .unwrap();
    let keys = test_encryption_keys();
    let (_, established, _) =
        create_registration_token_for_actor(&db, &actor, &org, "established", 10, 300)
            .await
            .unwrap();
    let (established, _, _) = register_node(&db, &keys, &established, None).await.unwrap();
    let (_, pending, _) =
        create_registration_token_for_actor(&db, &actor, &org, "departing-admin", 10, 300)
            .await
            .unwrap();
    let (legacy_id, legacy, _) =
        create_registration_token_for_actor(&db, &successor, &org, "legacy-admin", 10, 300)
            .await
            .unwrap();
    db.collection::<NodeRegistrationToken>(NODE_REG_TOKENS)
        .update_one(
            doc! {"_id":legacy_id},
            doc! {"$unset":{"issuer_user_id":""}},
        )
        .await
        .unwrap();
    // Legacy org rows do not infer an authenticated issuer from ownership.
    assert!(register_node(&db, &keys, &legacy, None).await.is_err());
    let (_, unrelated, _) =
        create_registration_token_for_actor(&db, &successor, &org, "remaining-admin", 10, 300)
            .await
            .unwrap();
    let actor_membership = org_service::get_active_membership(&db, &org, &actor)
        .await
        .unwrap()
        .unwrap();
    org_service::update_membership(&db, &actor_membership.id, Some(OrgRole::Member), None, None)
        .await
        .unwrap();
    assert!(register_node(&db, &keys, &pending, None).await.is_err());
    assert!(
        db.collection::<NodeRegistrationToken>(NODE_REG_TOKENS)
            .find_one(doc! {"token_hash":hash_token(&legacy)})
            .await
            .unwrap()
            .is_none()
    );
    let (other, _, _) = register_node(&db, &keys, &unrelated, None).await.unwrap();
    assert_eq!(other.user_id, org);
    assert!(
        db.collection::<Node>(NODES)
            .find_one(doc! {"_id":&established.id,"user_id":&org,"is_active":true})
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        create_registration_token_for_actor(&db, &actor, &org, "late-mint", 10, 300)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn redemption_held_at_kms_cannot_survive_issuer_revoke_or_personal_departure() {
    for depart in [false, true] {
        let Some(db) = connect_test_database("node_registration_held_revoke").await else {
            return;
        };
        let (actor, org) = transfer_fixture(&db).await;
        let successor = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&successor, UserType::Person))
            .await
            .unwrap();
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org, &successor, OrgRole::Admin, None))
            .await
            .unwrap();
        let (id, raw, _) =
            create_registration_token_for_actor(&db, &actor, &org, "held-register", 10, 300)
                .await
                .unwrap();
        let provider = Arc::new(HeldWrap {
            inner: LocalKeyProvider::new([0x11; 32], None),
            entered: tokio::sync::Semaphore::new(0),
            release: tokio::sync::Semaphore::new(0),
        });
        let held_keys = EncryptionKeys::with_provider(provider.clone());
        let redeem = register_node(&db, &held_keys, &raw, None);
        let revoke = async {
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                provider.entered.acquire(),
            )
            .await
            .unwrap()
            .unwrap()
            .forget();
            if depart {
                let mut tx = db.client().start_session().await.unwrap();
                tx.start_transaction()
                    .write_concern(mongodb::options::WriteConcern::majority())
                    .await
                    .unwrap();
                crate::services::org_membership_authority::fence_person_authority_loss(
                    &db, &actor, &mut tx, true,
                )
                .await
                .unwrap();
                db.collection::<User>(USERS)
                    .update_one(
                        doc! {"_id":&actor},
                        doc! {"$set":{"is_active":false,"platform_personal_deleting":true}},
                    )
                    .session(&mut tx)
                    .await
                    .unwrap();
                crate::services::transaction_service::commit(&mut tx)
                    .await
                    .unwrap();
            } else {
                org_service::revoke_membership(&db, &org, &actor)
                    .await
                    .unwrap();
            }
            provider.release.add_permits(1);
        };
        let (result, _) = tokio::join!(redeem, revoke);
        assert!(result.is_err());
        assert!(
            db.collection::<Node>(NODES)
                .find_one(doc! {"_id":&id})
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            db.collection::<NodeRegistrationToken>(NODE_REG_TOKENS)
                .find_one(doc! {"_id":&id})
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            create_registration_token_for_actor(&db, &actor, &org, "stale-mint", 10, 300)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn registration_and_transfer_preserve_secret_across_legacy_and_contextual_rows() {
    let Some(db) = connect_test_database("node_contextual_transfer").await else {
        return;
    };
    let keys = test_encryption_keys();
    let (actor, org) = transfer_fixture(&db).await;
    for legacy in [true, false] {
        // Exercise real registration, reserved identity and persisted format.
        let (_, token, _) = create_registration_token(
            &db,
            &actor,
            if legacy { "legacy" } else { "contextual" },
            10,
            300,
        )
        .await
        .unwrap();
        let (mut node, auth, signing) = register_node(&db, &keys, &token, None).await.unwrap();
        assert_eq!(node.signing_secret_context_version, Some(3));
        assert_eq!(hash_token(&auth), node.auth_token_hash);
        assert_eq!(
            *signing_secret_from_node(&keys, &node).await.unwrap(),
            hex::decode(&signing).unwrap()
        );
        if legacy {
            node.signing_secret_encrypted = Some(keys.encrypt(signing.as_bytes()).await.unwrap());
            node.signing_secret_context_version = None;
            db.collection::<Node>(NODES)
                .replace_one(doc! {"_id":&node.id}, &node)
                .await
                .unwrap();
        }
        transfer_node_owner(&db, &keys, &actor, &node.id, &org, 10)
            .await
            .unwrap();
        let stored = db
            .collection::<Node>(NODES)
            .find_one(doc! {"_id":&node.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.user_id, org);
        assert_eq!(stored.signing_secret_context_version, Some(3));
        assert_eq!(stored.signing_secret_hash, node.signing_secret_hash);
        assert_eq!(stored.auth_token_hash, node.auth_token_hash);
        assert_eq!(
            *signing_secret_from_node(&keys, &stored).await.unwrap(),
            hex::decode(&signing).unwrap()
        );
        let mut wrong_owner = stored.clone();
        wrong_owner.user_id = actor.clone();
        assert!(signing_secret_from_node(&keys, &wrong_owner).await.is_err());
        let before = bson::to_document(&stored).unwrap();
        assert!(matches!(
            transfer_node_owner_with_expected_state_version(
                &db,
                &keys,
                &actor,
                &node.id,
                &actor,
                10,
                Some(99)
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        let after = db
            .collection::<Node>(NODES)
            .find_one(doc! {"_id":&node.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            bson::to_document(&after).unwrap(),
            before,
            "rejected transfer preserves exact ciphertext and owner"
        );
    }
}

#[derive(Debug)]
struct HeldWrap {
    inner: LocalKeyProvider,
    entered: tokio::sync::Semaphore,
    release: tokio::sync::Semaphore,
}

#[tokio::test]
async fn migration_accepts_exact_legacy_array_storage_and_preserves_stale_source_fence() {
    let db = crate::test_utils::connect_transaction_test_database("node_array_migration").await;
    let keys = test_encryption_keys();
    let mut node = make_node("legacy-owner", "array-migration");
    let secret = "ab".repeat(32);
    node.signing_secret_hash = hash_token(&secret);
    let encrypted = keys.encrypt(secret.as_bytes()).await.unwrap();
    node.signing_secret_encrypted = Some(encrypted.clone());
    let mut source = bson::to_document(&node).unwrap();
    source.remove("signing_secret_context_version");
    source.insert(
        "signing_secret_encrypted",
        encrypted
            .into_iter()
            .map(|byte| bson::Bson::Int32(byte.into()))
            .collect::<Vec<_>>(),
    );
    db.collection::<bson::Document>(NODES)
        .insert_one(&source)
        .await
        .unwrap();
    assert_eq!(
        migrate_signing_secret_snapshot(&db, &keys, &source)
            .await
            .unwrap(),
        SecretMigrationOutcome::Rewritten
    );
    let migrated = db
        .collection::<Node>(NODES)
        .find_one(doc! {"_id":&node.id})
        .await
        .unwrap()
        .unwrap();
    verify_current_signing_secret(&keys, &migrated)
        .await
        .unwrap();
    assert_eq!(
        *signing_secret_from_node(&keys, &migrated).await.unwrap(),
        vec![0xab; 32]
    );
    assert_eq!(
        migrate_signing_secret_snapshot(&db, &keys, &source)
            .await
            .unwrap(),
        SecretMigrationOutcome::ConcurrentChange
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn migration_authenticates_current_rows_and_cas_preserves_concurrent_rotation() {
    let Some(db) = connect_test_database("node_secret_migration_cas").await else {
        return;
    };
    let keys = test_encryption_keys();
    let (actor, _) = transfer_fixture(&db).await;
    let mut node = make_node(&actor, "migration");
    let secret = "ef".repeat(32);
    node.signing_secret_hash = hash_token(&secret);
    node.signing_secret_encrypted = Some(keys.encrypt(secret.as_bytes()).await.unwrap());
    db.collection::<Node>(NODES)
        .insert_one(&node)
        .await
        .unwrap();
    assert!(verify_current_signing_secret(&keys, &node).await.is_err());
    assert_eq!(
        migrate_signing_secret_snapshot(&db, &keys, &bson::to_document(&node).unwrap())
            .await
            .unwrap(),
        SecretMigrationOutcome::Rewritten
    );
    let migrated = db
        .collection::<Node>(NODES)
        .find_one(doc! {"_id":&node.id})
        .await
        .unwrap()
        .unwrap();
    verify_current_signing_secret(&keys, &migrated)
        .await
        .unwrap();
    assert_eq!(migrated.auth_token_hash, node.auth_token_hash);
    assert_eq!(migrated.signing_secret_hash, node.signing_secret_hash);
    assert_eq!(
        migrated.updated_at.timestamp_millis(),
        node.updated_at.timestamp_millis()
    );
    let mut malformed = migrated.clone();
    malformed.signing_secret_encrypted = Some(b"\x03NYXmalformed-current-envelope".to_vec());
    assert!(
        verify_current_signing_secret(&keys, &malformed)
            .await
            .is_err()
    );
    assert!(
        migrate_signing_secret_snapshot(&db, &keys, &bson::to_document(&malformed).unwrap())
            .await
            .is_err()
    );
    let provider = Arc::new(HeldWrap {
        inner: LocalKeyProvider::new([0x11; 32], None),
        entered: tokio::sync::Semaphore::new(0),
        release: tokio::sync::Semaphore::new(0),
    });
    let held_keys = EncryptionKeys::with_provider(provider.clone());
    let migration_source = bson::to_document(&migrated).unwrap();
    let migration = migrate_signing_secret_snapshot(&db, &held_keys, &migration_source);
    let replacement = async {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            provider.entered.acquire(),
        )
        .await
        .unwrap()
        .unwrap()
        .forget();
        rotate_auth_token(&db, &keys, &actor, &node.id)
            .await
            .unwrap();
        let current = db
            .collection::<Node>(NODES)
            .find_one(doc! {"_id":&node.id})
            .await
            .unwrap()
            .unwrap();
        provider.release.add_permits(1);
        current
    };
    let (outcome, successor) = tokio::join!(migration, replacement);
    assert_eq!(outcome.unwrap(), SecretMigrationOutcome::ConcurrentChange);
    let current = db
        .collection::<Node>(NODES)
        .find_one(doc! {"_id":&node.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        bson::to_document(&current).unwrap(),
        bson::to_document(&successor).unwrap()
    );
    verify_current_signing_secret(&keys, &current)
        .await
        .unwrap();
}
#[async_trait::async_trait]
impl KeyProvider for HeldWrap {
    async fn wrap_dek(&self, dek: &[u8]) -> AppResult<WrappedKey> {
        self.entered.add_permits(1);
        self.release.acquire().await.unwrap().forget();
        self.inner.wrap_dek(dek).await
    }
    async fn unwrap_dek(&self, wrapped: &WrappedKey) -> AppResult<Zeroizing<Vec<u8>>> {
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
async fn rotation_held_after_acl_cannot_overwrite_successor_owner_or_credentials() {
    let Some(db) = connect_test_database("node_contextual_rotation_race").await else {
        return;
    };
    let keys = test_encryption_keys();
    let (actor, org) = transfer_fixture(&db).await;
    for transfer in [true, false] {
        let (_, token, _) = create_registration_token(
            &db,
            &actor,
            if transfer {
                "held-transfer"
            } else {
                "held-rotation"
            },
            10,
            300,
        )
        .await
        .unwrap();
        let (node, _, _) = register_node(&db, &keys, &token, None).await.unwrap();
        let provider = Arc::new(HeldWrap {
            inner: LocalKeyProvider::new([0x11; 32], None),
            entered: tokio::sync::Semaphore::new(0),
            release: tokio::sync::Semaphore::new(0),
        });
        let held_keys = EncryptionKeys::with_provider(provider.clone());
        let stale = rotate_auth_token(&db, &held_keys, &actor, &node.id);
        let successor = async {
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                provider.entered.acquire(),
            )
            .await
            .unwrap()
            .unwrap()
            .forget();
            if transfer {
                transfer_node_owner(&db, &keys, &actor, &node.id, &org, 10)
                    .await
                    .unwrap();
            } else {
                rotate_auth_token(&db, &keys, &actor, &node.id)
                    .await
                    .unwrap();
            }
            let stored = db
                .collection::<Node>(NODES)
                .find_one(doc! {"_id":&node.id})
                .await
                .unwrap()
                .unwrap();
            provider.release.add_permits(1);
            stored
        };
        let (stale, successor) = tokio::join!(stale, successor);
        assert!(matches!(stale, Err(AppError::NodeNotFound(_))));
        let final_row = db
            .collection::<Node>(NODES)
            .find_one(doc! {"_id":&node.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            bson::to_document(&final_row).unwrap(),
            bson::to_document(&successor).unwrap()
        );
        assert!(signing_secret_from_node(&keys, &final_row).await.is_ok());
    }
}

#[tokio::test]
async fn transfer_held_during_rebind_uses_live_name_and_rejects_successor_rotation() {
    let Some(db) = connect_test_database("node_contextual_transfer_race").await else {
        return;
    };
    let keys = test_encryption_keys();
    let (actor, org) = transfer_fixture(&db).await;
    for rotate in [false, true] {
        let name = if rotate {
            "transfer-rotate"
        } else {
            "transfer-rename"
        };
        let (_, token, _) = create_registration_token(&db, &actor, name, 10, 300)
            .await
            .unwrap();
        let (node, _, _) = register_node(&db, &keys, &token, None).await.unwrap();
        // A stale name would falsely reject the rename branch.
        if !rotate {
            db.collection::<Node>(NODES)
                .insert_one(make_node(&org, name))
                .await
                .unwrap();
        }
        let provider = Arc::new(HeldWrap {
            inner: LocalKeyProvider::new([0x11; 32], None),
            entered: tokio::sync::Semaphore::new(0),
            release: tokio::sync::Semaphore::new(0),
        });
        let held_keys = EncryptionKeys::with_provider(provider.clone());
        let stale = transfer_node_owner(&db, &held_keys, &actor, &node.id, &org, 10);
        let successor = async {
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                provider.entered.acquire(),
            )
            .await
            .unwrap()
            .unwrap()
            .forget();
            if rotate {
                rotate_auth_token(&db, &keys, &actor, &node.id)
                    .await
                    .unwrap();
            } else {
                db.collection::<Node>(NODES)
                    .update_one(
                        doc! {"_id":&node.id},
                        doc! {"$set":{"name":"renamed-during-rebind"}},
                    )
                    .await
                    .unwrap();
            }
            let row = db
                .collection::<Node>(NODES)
                .find_one(doc! {"_id":&node.id})
                .await
                .unwrap()
                .unwrap();
            provider.release.add_permits(1);
            row
        };
        let (result, successor) = tokio::join!(stale, successor);
        let current = db
            .collection::<Node>(NODES)
            .find_one(doc! {"_id":&node.id})
            .await
            .unwrap()
            .unwrap();
        if rotate {
            assert!(matches!(result, Err(AppError::Conflict(_))));
            assert_eq!(
                bson::to_document(&current).unwrap(),
                bson::to_document(&successor).unwrap()
            );
        } else {
            result.unwrap();
            assert_eq!(current.user_id, org);
            assert_eq!(current.name, "renamed-during-rebind");
        }
        signing_secret_from_node(&keys, &current).await.unwrap();
    }
}

#[tokio::test]
async fn staged_node_writes_preserve_legacy_registration_rotation_transfer_and_managed_activation()
{
    let Some(db) = connect_test_database("node_staged_encrypted_writes").await else {
        return;
    };
    let mut keys = test_encryption_keys();
    keys.set_contextual_record_writes(false);
    let (actor, org) = transfer_fixture(&db).await;
    let (_, registration, _) = create_registration_token(&db, &actor, "staged-node", 10, 300)
        .await
        .unwrap();
    let (node, _, signing) = register_node(&db, &keys, &registration, None)
        .await
        .unwrap();
    assert_eq!(node.signing_secret_context_version, None);
    assert_eq!(
        keys.decrypt(node.signing_secret_encrypted.as_deref().unwrap())
            .await
            .unwrap(),
        signing.as_bytes()
    );
    let (_, rotated) = rotate_auth_token(&db, &keys, &actor, &node.id)
        .await
        .unwrap();
    let node = db
        .collection::<Node>(NODES)
        .find_one(doc! {"_id":&node.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(node.signing_secret_context_version, None);
    assert_eq!(
        keys.decrypt(node.signing_secret_encrypted.as_deref().unwrap())
            .await
            .unwrap(),
        rotated.as_bytes()
    );
    transfer_node_owner(&db, &keys, &actor, &node.id, &org, 10)
        .await
        .unwrap();
    let moved = db
        .collection::<Node>(NODES)
        .find_one(doc! {"_id":&node.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(moved.user_id, org);
    assert_eq!(moved.signing_secret_context_version, None);
    assert_eq!(
        keys.decrypt(moved.signing_secret_encrypted.as_deref().unwrap())
            .await
            .unwrap(),
        rotated.as_bytes()
    );
    let device = create_for_device(
        &db,
        &keys,
        DeviceNodeInput {
            user_id: &actor,
            api_key_id: "device-key",
            hw_id: "hardware-id",
            label: "staged-device",
            device_pubkey: None,
            provisioning_source: "device",
        },
    )
    .await
    .unwrap();
    assert_eq!(device.signing_secret_context_version, None);
    assert_eq!(
        keys.decrypt(device.signing_secret_encrypted.as_deref().unwrap())
            .await
            .unwrap()
            .len(),
        64
    );
    db.collection::<Node>(NODES)
        .update_one(
            doc! {"_id":&device.id},
            doc! {"$set":{"metadata.provisioning_source":"remote_machine"}},
        )
        .await
        .unwrap();
    let material = prepare_managed_credentials(&keys, &device.id, &actor)
        .await
        .unwrap();
    assert_eq!(material.signing_secret_context_version, None);
    let mut tx = db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    activate_managed_credentials(
        &db,
        &mut tx,
        &device.id,
        &actor,
        &hash_token(&material.auth_token),
        &hash_token(&material.signing_secret),
        &material.signing_secret_encrypted,
        material.signing_secret_context_version,
        Utc::now(),
    )
    .await
    .unwrap();
    tx.commit_transaction().await.unwrap();
    let managed = db
        .collection::<Node>(NODES)
        .find_one(doc! {"_id":&device.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(managed.signing_secret_context_version, None);
    assert_eq!(
        keys.decrypt(managed.signing_secret_encrypted.as_deref().unwrap())
            .await
            .unwrap(),
        material.signing_secret.as_bytes()
    );
    db.drop().await.unwrap();
}
