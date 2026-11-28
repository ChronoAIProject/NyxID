use super::*;
use crate::test_utils::{connect_transaction_test_database, test_app_config, test_auth_user};

#[tokio::test]
async fn unenrolled_admission_is_one_indexed_read_without_lifecycle_environment() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};

    let commands = Arc::new(Mutex::new(Vec::new()));
    let captured = commands.clone();
    let db = crate::test_utils::connect_test_database_with_command_handler(
        "unenrolled_admission",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                captured.lock().unwrap().push(event.command);
            }
        }),
    )
    .await
    .unwrap();
    commands.lock().unwrap().clear();

    admit_startup(&db, &test_app_config()).await.unwrap();

    {
        let commands = commands.lock().unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].get_str("find"), Ok(HEAD));
        assert_eq!(
            commands[0]
                .get_document("filter")
                .and_then(|filter| filter.get_str("_id")),
            Ok(SINGLETON)
        );
    }
    db.drop().await.unwrap();
}

fn recording(rev: i64) -> Recording {
    Recording {
        request_id: fresh(),
        expected_revision: rev,
        owner: "recovery-team".into(),
        evidence_ref: fresh(),
    }
}
fn input(last: &Receipt, change: Change) -> Append {
    Append {
        recording: recording(last.revision),
        scope: last.scope.clone(),
        change,
    }
}
fn enroll_request() -> Enroll {
    Enroll {
        recording: recording(0),
        deployment_revision: "release-1".into(),
    }
}
fn actor() -> AuditActor {
    AuditActor::from_auth_user(&test_auth_user(&fresh()))
}
fn dependency(kind: Inventory, parents: &[String]) -> Change {
    Change::OpenDependency {
        inventory: kind,
        parents: parents.iter().cloned().collect(),
        formats: ["contextual_v3".to_string()].into(),
    }
}

#[tokio::test]
async fn existing_rings_verify_exact_possession_and_independent_hmac_adoption() {
    let db = connect_transaction_test_database("ring_enrollment").await;
    let actor = actor();
    let mut config = test_app_config();
    config.contextual_record_writes = false;
    let request = enroll_request();
    let first = enroll(&db, &config, &actor, &request).await.unwrap();
    let state = status(&db, &config).await.unwrap().unwrap();
    assert!(!state.rotation_open && !state.independent_hmac_file_verified);
    assert_eq!(first.scope.keys.len(), 1);
    assert_eq!(
        enroll(&db, &config, &actor, &request)
            .await
            .unwrap()
            .entry_id,
        first.entry_id
    );
    let current = input(&first, Change::VerifyRing);
    append(&db, &config, &actor, &current).await.unwrap();
    let ring_db = connect_transaction_test_database("existing_overlap").await;
    config.encryption_key_previous = Some(config.encryption_key.take().unwrap());
    config.encryption_key = Some(hex::encode(rand::random::<[u8; 32]>()));
    let overlap = enroll(&ring_db, &config, &actor, &enroll_request())
        .await
        .unwrap();
    let state = status(&ring_db, &config).await.unwrap().unwrap();
    assert!(state.rotation_open);
    assert_eq!(state.keys.len(), 2);
    let check = input(&overlap, Change::VerifyRing);
    let mut swapped = config.clone();
    std::mem::swap(
        &mut swapped.encryption_key,
        &mut swapped.encryption_key_previous,
    );
    assert!(append(&ring_db, &swapped, &actor, &check).await.is_err());
    assert!(append(&ring_db, &config, &actor, &check).await.is_ok());
    let mut domains = std::array::from_fn(|_| zeroize::Zeroizing::new(rand::random::<[u8; 32]>()));
    domains[2].copy_from_slice(audit_service::audit_chain_hmac_key().unwrap());
    let hmac = crate::crypto::hmac_lifecycle::HmacKeyset(domains);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("independent");
    hmac.export_new(&path).unwrap();
    hmac_lifecycle_service::ensure_consistent(&db, &hmac, Some(&hmac))
        .await
        .unwrap();
    assert!(
        !status(&db, &config)
            .await
            .unwrap()
            .unwrap()
            .independent_hmac_file_verified
    );
    config.hmac_keyset_path = Some(path.to_str().unwrap().into());
    assert!(
        status(&db, &config)
            .await
            .unwrap()
            .unwrap()
            .independent_hmac_file_verified
    );
    let other = directory.path().join("changed");
    hmac.with_changed_domain("cli_pairing")
        .export_new(&other)
        .unwrap();
    config.hmac_keyset_path = Some(other.to_str().unwrap().into());
    assert!(
        !status(&db, &config)
            .await
            .unwrap()
            .unwrap()
            .independent_hmac_file_verified
    );
}

#[tokio::test]
async fn inventory_requires_positive_reviews_and_closes_pitr_children_before_parents() {
    use Inventory::*;
    let db = connect_transaction_test_database("lifecycle_inventory").await;
    let config = test_app_config();
    let actor = actor();
    let mut last = enroll(&db, &config, &actor, &enroll_request())
        .await
        .unwrap();
    let recovery = || Change::Attest {
        assertion: Assertion::RecoveryDependenciesClosed,
        affirmed: true,
        valid_until_ms: None,
    };
    assert!(
        append(&db, &config, &actor, &input(&last, recovery()))
            .await
            .is_err()
    );
    for kind in [
        FullBackup,
        IncrementalBackup,
        Pitr,
        Clone,
        Archive,
        RestoreDrill,
        Rollback,
        Unknown,
    ] {
        last = append(
            &db,
            &config,
            &actor,
            &input(
                &last,
                Change::ReviewInventory {
                    inventory: kind,
                    disposition: Review::ReviewedEmpty,
                },
            ),
        )
        .await
        .unwrap();
    }
    last = append(&db, &config, &actor, &input(&last, recovery()))
        .await
        .unwrap();
    let backup = append(
        &db,
        &config,
        &actor,
        &input(&last, dependency(FullBackup, &[])),
    )
    .await
    .unwrap();
    let pitr = append(
        &db,
        &config,
        &actor,
        &input(
            &backup,
            dependency(Pitr, std::slice::from_ref(&backup.entry_id)),
        ),
    )
    .await
    .unwrap();
    let close = |id: &str| Change::CloseDependency {
        dependency_id: id.into(),
        resolution: Resolution::TestedSurvivorRestore,
    };
    assert!(
        append(&db, &config, &actor, &input(&pitr, close(&backup.entry_id)))
            .await
            .is_err()
    );
    assert!(
        append(
            &db,
            &config,
            &actor,
            &input(
                &pitr,
                Change::ReviewInventory {
                    inventory: Pitr,
                    disposition: Review::ReviewedEmpty
                }
            )
        )
        .await
        .is_err()
    );
    let closed = append(&db, &config, &actor, &input(&pitr, close(&pitr.entry_id)))
        .await
        .unwrap();
    last = append(
        &db,
        &config,
        &actor,
        &input(&closed, close(&backup.entry_id)),
    )
    .await
    .unwrap();
    let late = append(
        &db,
        &config,
        &actor,
        &input(
            &last,
            dependency(IncrementalBackup, std::slice::from_ref(&backup.entry_id)),
        ),
    )
    .await
    .unwrap();
    assert!(
        append(&db, &config, &actor, &input(&late, recovery()))
            .await
            .is_err()
    );
    assert_eq!(
        dependencies(
            &db,
            Page {
                after: None,
                limit: None
            }
        )
        .await
        .unwrap()
        .len(),
        3
    );
}

#[tokio::test]
async fn concurrent_edits_and_old_clone_evidence_cannot_cross_a_restore_epoch() {
    let db = connect_transaction_test_database("lifecycle_epoch").await;
    let cfg = test_app_config();
    let a = actor();
    let b = actor();
    let enrolled = enroll(&db, &cfg, &a, &enroll_request()).await.unwrap();
    let claim = || Change::Attest {
        assertion: Assertion::RestoreContinuity,
        affirmed: true,
        valid_until_ms: None,
    };
    let left = input(&enrolled, claim());
    let right = input(&enrolled, claim());
    let (x, y) = tokio::join!(append(&db, &cfg, &a, &left), append(&db, &cfg, &b, &right));
    assert_ne!(x.is_ok(), y.is_ok());
    let winner = x.or(y).unwrap();
    let old = input(&winner, claim());
    let advance = input(&winner, Change::AdvanceRestoreEpoch);
    let restored = append(&db, &cfg, &a, &advance).await.unwrap();
    assert_ne!(restored.scope.restore_epoch, enrolled.scope.restore_epoch);
    let mut replay = old;
    replay.recording.expected_revision = restored.revision;
    assert!(append(&db, &cfg, &a, &replay).await.is_err());
    assert!(status(&db, &cfg).await.unwrap().unwrap().current.is_empty());
    assert_eq!(
        append(&db, &cfg, &a, &advance).await.unwrap().revision,
        restored.revision
    );
    assert!(append(&db, &cfg, &b, &advance).await.is_err());
    let history = evidence(
        &db,
        Page {
            after: None,
            limit: Some(3),
        },
    )
    .await
    .unwrap();
    assert_eq!(history.len(), 3);
    assert_eq!(
        history[1].receipt.scope.restore_epoch,
        enrolled.scope.restore_epoch
    );
}

mod router;

#[tokio::test]
async fn copied_lifecycle_detects_a_clone_and_census_loses_its_epoch() {
    use crate::services::encrypted_record_census::{Census, Scope as CensusScope};
    let db = connect_transaction_test_database("lifecycle_original").await;
    let config = test_app_config();
    let a = actor();
    let original = enroll(&db, &config, &a, &enroll_request()).await.unwrap();
    let scope = CensusScope {
        registry_digest: "fixture".into(),
        deployment_revision: original.scope.deployment_revision.clone(),
        restore_epoch: original.scope.restore_epoch.clone(),
        namespaces: ["fixture_rows".into()].into(),
    };
    let mut keys = crate::test_utils::test_encryption_keys();
    keys.set_contextual_record_writes(true);
    let (census, _) = Census::start(&db, &keys, scope).await.unwrap();

    let copy = connect_transaction_test_database("lifecycle_clone").await;
    for name in [HEAD, ENTRIES, DEPENDENCIES] {
        let mut cursor = db.collection::<Document>(name).find(doc! {}).await.unwrap();
        copy.create_collection(name).await.unwrap();
        while let Some(row) = cursor.try_next().await.unwrap() {
            copy.collection::<Document>(name)
                .insert_one(row)
                .await
                .unwrap();
        }
    }
    let clone = status(&copy, &config).await.unwrap().unwrap();
    assert!(clone.restore_required);
    assert_eq!(clone.revision, original.revision + 1);
    assert!(
        append(&copy, &config, &a, &input(&original, Change::VerifyRing))
            .await
            .is_err()
    );
    let system_audit = copy.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .count_documents(doc!{"event_data.kind":"restore_detected","user_id":bson::Bson::Null,"entry_hash":{"$type":"string"}}).await.unwrap();
    assert_eq!(system_audit, 1);
    let repair = Append {
        recording: recording(clone.revision),
        scope: clone.scope,
        change: Change::AdvanceRestoreEpoch,
    };
    let advanced = append(&copy, &config, &a, &repair).await.unwrap();
    assert_ne!(advanced.scope.restore_epoch, original.scope.restore_epoch);
    assert!(
        !status(&copy, &config)
            .await
            .unwrap()
            .unwrap()
            .restore_required
    );
    assert!(census.request_cut().await.is_ok());
    let released = append(
        &db,
        &config,
        &a,
        &input(
            &original,
            Change::SetDeploymentRevision {
                deployment_revision: "release-2".into(),
            },
        ),
    )
    .await
    .unwrap();
    let returned = append(
        &db,
        &config,
        &a,
        &input(
            &released,
            Change::SetDeploymentRevision {
                deployment_revision: original.scope.deployment_revision.clone(),
            },
        ),
    )
    .await
    .unwrap();
    assert!(returned.scope == original.scope);
    assert!(census.request_cut().await.is_err());
    append(
        &db,
        &config,
        &a,
        &input(&returned, Change::AdvanceRestoreEpoch),
    )
    .await
    .unwrap();
    assert!(census.request_cut().await.is_err());
}

#[tokio::test]
async fn reopening_a_credential_withdraws_its_review_and_blocks_an_affirmative_claim() {
    let db = connect_transaction_test_database("reopen_dependency").await;
    let cfg = test_app_config();
    let admin = actor();
    let enrollment = enroll(&db, &cfg, &admin, &enroll_request()).await.unwrap();
    let open = append(
        &db,
        &cfg,
        &admin,
        &input(&enrollment, dependency(Inventory::DatabaseCredential, &[])),
    )
    .await
    .unwrap();
    let closed = append(
        &db,
        &cfg,
        &admin,
        &input(
            &open,
            Change::CloseDependency {
                dependency_id: open.entry_id.clone(),
                resolution: Resolution::RevokedOrRotated,
            },
        ),
    )
    .await
    .unwrap();
    let claim = || Change::Attest {
        assertion: Assertion::DatabaseAccessRevoked,
        affirmed: true,
        valid_until_ms: None,
    };
    let affirmed = append(&db, &cfg, &admin, &input(&closed, claim()))
        .await
        .unwrap();
    let reopened = append(
        &db,
        &cfg,
        &admin,
        &input(
            &affirmed,
            Change::ReopenDependency {
                dependency_id: open.entry_id,
            },
        ),
    )
    .await
    .unwrap();
    assert!(
        append(&db, &cfg, &admin, &input(&reopened, claim()))
            .await
            .is_err()
    );
    let head = status(&db, &cfg).await.unwrap().unwrap();
    assert_eq!(head.revision, reopened.revision);
    assert!(head.current["assertion_database_access_revoked"].is_empty());
}
mod mixed;
