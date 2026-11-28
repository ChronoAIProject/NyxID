use super::*;
use crate::{
    services::encrypted_record_census::{report, tests::with_history},
    test_utils::{connect_transaction_test_database, test_app_config, test_auth_user},
};

struct Fixture {
    config: AppConfig,
    actor: AuditActor,
    request: Request,
    _files: tempfile::TempDir,
}

fn recording(revision: i64) -> Recording {
    Recording {
        expected_revision: revision,
        request_id: fresh(),
        owner: "recovery-team".into(),
        evidence_ref: fresh(),
    }
}
async fn fixture(db: &Database) -> Fixture {
    let mut config = test_app_config();
    config.encryption_key_previous = Some("5a".repeat(32));
    config.contextual_record_writes = true;
    config.allow_legacy_record_reads = false;
    let files = match std::env::var_os("NYXID_RETIREMENT_CRASH_FIXTURE") {
        Some(parent) => tempfile::tempdir_in(parent).unwrap(),
        None => tempfile::tempdir().unwrap(),
    };
    let path = files.path().join("independent");
    let mut domains = std::array::from_fn(|_| zeroize::Zeroizing::new(rand::random::<[u8; 32]>()));
    domains[2].copy_from_slice(audit_service::audit_chain_hmac_key().unwrap());
    let hmac = crate::crypto::hmac_lifecycle::HmacKeyset(domains);
    hmac.export_new(&path).unwrap();
    hmac_lifecycle_service::ensure_consistent(db, &hmac, Some(&hmac))
        .await
        .unwrap();
    config.hmac_keyset_path = Some(path.to_str().unwrap().into());
    let actor = AuditActor::from_auth_user(&test_auth_user(&fresh()));
    let mut receipt = enroll(
        db,
        &config,
        &actor,
        &Enroll {
            recording: recording(0),
            deployment_revision: "release-9".into(),
        },
    )
    .await
    .unwrap();
    db.collection::<Document>("platform_native_entry_receipts")
        .create_index(IndexModel::builder().keys(doc! {"expires_at":1}).build())
        .await
        .unwrap();
    db.collection::<Document>("platform_entry_migrations")
        .insert_one(doc! {"_id":"native-entry-v2","state":"ready","schema_version":2})
        .await
        .unwrap();
    fleet::observe_retirement_test_replica(db, cohort(&receipt.scope)).await;
    db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &actor.user_id,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    for name in [
        "saved_logins",
        "platform_native_reply_handles",
        "downstream_services",
    ] {
        db.create_collection(name).await.unwrap();
    }
    let keys = EncryptionKeys::from_config(&config);
    let generation = report::start(db, &keys, &config).await.unwrap().generation;
    let head = status(db, &config).await.unwrap().unwrap();
    let target = Target {
        generation: generation.clone(),
        retiring: head
            .keys
            .iter()
            .find(|k| k.role == "previous")
            .unwrap()
            .key_ref
            .clone(),
        survivor: head
            .keys
            .iter()
            .find(|k| k.role == "primary")
            .unwrap()
            .key_ref
            .clone(),
    };
    for change in guards::INVENTORIES
        .into_iter()
        .map(|inventory| Change::ReviewInventory {
            inventory,
            disposition: Review::ReviewedEmpty,
        })
        .chain(
            guards::ASSERTIONS
                .into_iter()
                .map(|(_, assertion)| Change::Attest {
                    assertion,
                    affirmed: true,
                    valid_until_ms: None,
                }),
        )
    {
        receipt = append(
            db,
            &config,
            &actor,
            &Append {
                recording: recording(receipt.revision),
                scope: receipt.scope,
                change,
            },
        )
        .await
        .unwrap();
    }
    for _ in 0..128 {
        if report::advance(db, &keys, &config, &generation)
            .await
            .unwrap()
            .complete
        {
            break;
        }
    }
    let report = evaluate(db, &config, &target).await.unwrap();
    assert_eq!(report.blockers, BTreeSet::from([LOAD_BLOCKER]));
    let request = Request {
        recording: recording(receipt.revision),
        expected_step_revision: receipt.revision,
        target,
        report_revision: report.report_revision.unwrap(),
    };
    Fixture {
        config,
        actor,
        request,
        _files: files,
    }
}

async fn head(db: &Database) -> Document {
    db.collection::<Document>(HEAD)
        .find_one(doc! {"_id":SINGLETON})
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn witnessed_consumption_replay_and_exact_survivor_admission() {
    let db = connect_transaction_test_database("retirement_witnesses").await;
    with_history(&db,300,async{
        let mut f=fixture(&db).await;
        let err=advance(&db,&f.config,&f.actor,&f.request).await.err().unwrap();
        assert!(err.to_string().contains(LOAD_BLOCKER));
        assert!(!head(&db).await.contains_key("retirement_state"));
        let mut survivor=f.config.clone();
        survivor.encryption_key_previous=None;
        for phase in [Phase::Decision,Phase::Approved,Phase::Consume,Phase::Consumed,Phase::Published]{
            let dual_ticket = admit_enrolled(&db,&db,&f.config,f.request.expected_step_revision,"release-9").await.unwrap();
            assert!(admit_enrolled(&db,&db,&survivor,f.request.expected_step_revision,"release-9").await.is_err());
            let progress=step(&db,&f.config,&f.actor,&f.request,&CertifiedRelease).await.unwrap();
            assert_eq!(progress.phase,phase);
            let replay=step(&db,&f.config,&f.actor,&f.request,&CertifiedRelease).await.unwrap();
            assert_eq!((replay.phase,replay.revision),(phase,progress.revision));
            let row=head(&db).await;
            assert_eq!(row.contains_key("published_retirement"),phase==Phase::Published);
            if phase==Phase::Published {
                assert!(Initialization::begin(&db,Some(&dual_ticket)).await.is_err());
                assert!(admit_enrolled(&db,&db,&f.config,progress.revision,"release-9").await.is_err());
                for _ in 0..2 {
                    let current=head(&db).await;
                    let ticket=admit_enrolled(&db,&db,&survivor,current.get_i64("revision").unwrap(),"release-9").await.unwrap();
                    let init=Initialization::begin(&db,Some(&ticket)).await.unwrap();
                    init.finish(&db).await.unwrap();
                }
            } else {
                let scope:Scope=field(&row,"scope").unwrap();
                let proofs:Vec<proof::Proof>=field(&row,"proofs").unwrap();
                assert!(proof::verify(&scope.deployment_id,&proof::slots(&survivor).await.unwrap(),&proofs).await.is_err());
            }
            f.request.expected_step_revision=progress.revision;
        }
        let row=head(&db).await;
        for _ in 0..2 {admit(&db,&survivor,&row).await.unwrap();}
        let mut missing_hmac = survivor.clone();
        missing_hmac.hmac_keyset_path=None;
        assert!(admit(&db,&missing_hmac,&row).await.err().unwrap().to_string().contains("independent_hmac_unverified"));
        let ring = census_ring(&db,&survivor).await.unwrap();
        assert_eq!(ring.proofs.len(), 1);
        assert_eq!(ring.proofs[0].key.key_ref,f.request.target.survivor);
        let scope:Scope=field(&row,"scope").unwrap();
        append(&db,&survivor,&f.actor,&Append{recording:recording(row.get_i64("revision").unwrap()),scope,change:Change::VerifyRing}).await.unwrap();
        assert!(admission_reader(&f.config,&row).await.is_err());
        assert!(admit(&db,&f.config,&row).await.is_err());
        let latest=head(&db).await;
        let revision=latest.get_i64("revision").unwrap();
        let ticket=admit_enrolled(&db,&db,&survivor,revision,"release-9").await.unwrap();
        let mut retired_scope:Scope=field(&latest,"scope").unwrap();
        retired_scope.keys.insert(f.request.target.retiring.clone());
        let refused=Append{recording:recording(revision),scope:retired_scope,change:Change::OpenDependency{inventory:Inventory::FullBackup,parents:Default::default(),formats:["v3".into()].into()}};
        assert!(append(&db,&survivor,&f.actor,&refused).await.is_err());
        // An epoch advance must defeat a survivor whose bootstrap proof just finished.
        let new_epoch=Append{recording:recording(revision),scope:field(&latest,"scope").unwrap(),change:Change::AdvanceRestoreEpoch};
        append(&db,&survivor,&f.actor,&new_epoch).await.unwrap();
        assert!(Initialization::begin(&db,Some(&ticket)).await.is_err());
        assert!(admit_enrolled(&db,&db,&survivor,revision+1,"release-9").await.is_err());
        let audit=db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME);
        assert_eq!(audit.count_documents(doc!{"event_type":"encryption_key_lifecycle.retirement","event_data.phase":"published"}).await.unwrap(),1);
        fleet::stop_test_replica(&db).await;
    }).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn each_evidence_class_and_immutable_partition_is_required_at_consumption() {
    let db = connect_transaction_test_database("retirement_evidence").await;
    with_history(&db, 300, async {
        let f = fixture(&db).await;
        let baseline = head(&db).await;
        let scope: Scope = field(&baseline, "scope").unwrap();
        for (reason, _) in guards::ASSERTIONS {
            let path = format!("current.assertion_{reason}");
            let coll = db.collection::<Document>(HEAD);
            coll.update_one(doc! {"_id":SINGLETON}, doc! {"$unset":{&path:""}})
                .await
                .unwrap();
            let report = evaluate(&db, &f.config, &f.request.target).await.unwrap();
            assert_eq!(report.blockers, BTreeSet::from([LOAD_BLOCKER, reason]));
            coll.replace_one(doc! {"_id":SINGLETON}, baseline.clone())
                .await
                .unwrap();
        }
        let reports = db.collection::<Document>(report::REPORTS);
        let root = report::retirement::test_first_published_root(
            &db,
            &EncryptionKeys::from_config(&f.config),
            &f.request.target.generation,
        )
        .await;
        let part = reports
            .find_one(doc! {"_id":{"generation":&f.request.target.generation,"hash":root}})
            .await
            .unwrap()
            .unwrap();
        let id = part.get("_id").unwrap();
        for absent in [false, true] {
            if absent {
                reports.delete_one(doc! {"_id":id}).await.unwrap();
            } else {
                reports
                    .update_one(doc! {"_id":id}, doc! {"$set":{"partition":"corrupt"}})
                    .await
                    .unwrap();
            }
            let result = evaluate(&db, &f.config, &f.request.target).await.unwrap();
            assert_eq!(
                result.blockers,
                BTreeSet::from([LOAD_BLOCKER, "census_evidence_unavailable"])
            );
            assert!(
                step(&db, &f.config, &f.actor, &f.request, &CertifiedRelease)
                    .await
                    .is_err()
            );
            reports
                .replace_one(doc! {"_id":id}, part.clone())
                .upsert(true)
                .await
                .unwrap();
        }
        let mut request = f.request;
        let first = step(&db, &f.config, &f.actor, &request, &CertifiedRelease)
            .await
            .unwrap();
        request.expected_step_revision = first.revision;
        let edit = append(
            &db,
            &f.config,
            &f.actor,
            &Append {
                scope,
                recording: recording(first.revision),
                change: Change::OpenDependency {
                    inventory: Inventory::FullBackup,
                    formats: ["v3".into()].into(),
                    parents: Default::default(),
                },
            },
        )
        .await
        .unwrap();
        assert!(
            step(&db, &f.config, &f.actor, &request, &CertifiedRelease)
                .await
                .is_err()
        );
        assert_eq!(head(&db).await.get_i64("revision"), Ok(edit.revision));
        assert!(
            evaluate(&db, &f.config, &request.target)
                .await
                .unwrap()
                .blockers
                .contains("dependency_open")
        );
        fleet::stop_test_replica(&db).await;
    })
    .await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn lease_loss_at_every_transition_and_response_loss_after_every_commit() {
    use crate::services::durable_wake_tests::{drop_after, resume_after};
    let db = connect_transaction_test_database("retirement_crash").await;
    with_history(&db,300,async {
        let mut f=fixture(&db).await;
        for phase in [Phase::Decision,Phase::Approved,Phase::Consume,Phase::Consumed,Phase::Published] {
            let previous=f.request.expected_step_revision;
            let outcome=async {assert!(step(&db,&f.config,&f.actor,&f.request,&CertifiedRelease).await.is_err());};
            resume_after("retirement_claimed",outcome,async {
                let changed=db.collection::<Document>(report::REPORTS).update_one(
                    doc!{"_id":&f.request.target.generation,"claim":{"$exists":true}},
                    doc!{"$unset":{"claim":""},"$set":{"lease_until":bson::DateTime::from_millis(0)}}).await.unwrap();
                assert_eq!(changed.modified_count,1);
            }).await;
            assert_eq!(head(&db).await.get_i64("revision"),Ok(previous));
            drop_after("retirement_committed",step(&db,&f.config,&f.actor,&f.request,&CertifiedRelease)).await;
            let recovered=step(&db,&f.config,&f.actor,&f.request,&CertifiedRelease).await.unwrap();
            assert_eq!(recovered.phase,phase);
            assert_eq!(recovered.revision,previous+1);
            f.request.expected_step_revision=recovered.revision;
        }
        fleet::stop_test_replica(&db).await;
    }).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn stale_source_copy_and_current_writes_across_all_markers() {
    use crate::services::{durable_wake_tests::resume_after, saved_login_service as logins};
    let db = connect_transaction_test_database("retirement_writers").await;
    with_history(&db, 300, async {
        let mut f = fixture(&db).await;
        let dual = EncryptionKeys::from_config(&f.config);
        let mut old = f.config.clone();
        old.encryption_key = old.encryption_key_previous.take();
        let old = EncryptionKeys::from_config(&old);
        let owner = &f.actor.user_id;

        let input = || logins::Input {
            label: "retirement".into(),
            allowed_origins: vec!["https://example.test".into()],
            username: "fixture".to_owned().into(),
            password: Some("fixture".to_owned().into()),
            totp_secret: None,
            confirm_each_sign_in: false,
        };
        let row = logins::put(&db, &old, owner, owner, None, input())
            .await
            .unwrap();
        let stale = db
            .collection::<Document>("saved_logins")
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        logins::put(&db, &dual, owner, owner, Some(&row.id), input())
            .await
            .unwrap();
        for phase in [
            Phase::Decision,
            Phase::Approved,
            Phase::Consume,
            Phase::Consumed,
            Phase::Published,
        ] {
            let work = async {
                let result = step(&db, &f.config, &f.actor, &f.request, &CertifiedRelease)
                    .await
                    .unwrap();
                assert_eq!(result.phase, phase);
            };
            resume_after("retirement_claimed", work, async {
                logins::put(&db, &dual, owner, owner, Some(&row.id), input())
                    .await
                    .unwrap();
                assert!(
                    !logins::migrate_secret_snapshot(&db, &dual, &stale)
                        .await
                        .unwrap()
                );
                // Create and delete the pair before the transaction's marker.
                // The production cleanup path must preserve the survivor proof.
                let parent = fresh();
                let child = fresh();
                let expired = bson::DateTime::from_millis(1);
                let parents = db.collection::<Document>("platform_native_entry_receipts");
                let handles = db.collection::<Document>("platform_native_reply_handles");
                parents
                    .insert_one(doc! {"_id":&parent,"expires_at":expired})
                    .await
                    .unwrap();
                handles
                    .insert_one(doc! {"_id":&child,"receipt_id":&parent,"expires_at":expired})
                    .await
                    .unwrap();
                assert_eq!(
                    crate::services::platform_entry_receipts::cleanup_expired(&db, 1)
                        .await
                        .unwrap(),
                    1
                );
                assert!(
                    handles
                        .find_one(doc! {"_id":&child})
                        .await
                        .unwrap()
                        .is_none()
                );
            })
            .await;
            f.request.expected_step_revision += 1;
        }
        let mut survivor = f.config.clone();
        survivor.encryption_key_previous = None;
        let ticket = admit_enrolled(
            &db,
            &db,
            &survivor,
            f.request.expected_step_revision,
            "release-9",
        )
        .await
        .unwrap();
        let current = EncryptionKeys::from_config(&survivor);
        logins::put(&db, &current, owner, owner, Some(&row.id), input())
            .await
            .unwrap();
        let live = db.collection::<Document>("saved_logins");
        let before = live.find_one(doc! {"_id":&row.id}).await.unwrap().unwrap();
        // B alone cannot open the stale A source to construct a replacement.
        assert!(
            logins::migrate_secret_snapshot(&db, &current, &stale)
                .await
                .is_err()
        );
        let after = live.find_one(doc! {"_id":&row.id}).await.unwrap().unwrap();
        assert_eq!(
            bson::to_vec(&before).unwrap(),
            bson::to_vec(&after).unwrap()
        );
        let mut tx = db.client().start_session().await.unwrap();
        tx.start_transaction().await.unwrap();
        assert!(
            invalidate_in_transaction(&db, &mut tx, "companion_removed")
                .await
                .is_err()
        );
        tx.abort_transaction().await.unwrap();
        let init = Initialization::begin(&db, Some(&ticket)).await.unwrap();
        init.finish(&db).await.unwrap();
        fleet::stop_test_replica(&db).await;
    })
    .await;
    db.drop().await.unwrap();
}

#[cfg(unix)]
mod crash;

mod races;

mod blockers;
