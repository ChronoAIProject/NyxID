use super::*;

async fn reasons(db: &Database, f: &Fixture) -> BTreeSet<&'static str> {
    evaluate(db, &f.config, &f.request.target)
        .await
        .unwrap()
        .blockers
}

#[tokio::test]
async fn environmental_blockers_are_independent_of_release_certification() {
    let db = connect_transaction_test_database("retire_blockers").await;
    with_history(&db, 300, async {
        let mut f = fixture(&db).await;
        let lifecycle = db.collection::<Document>(HEAD);
        let original = head(&db).await;
        for (update, expected) in [
            (
                doc! {"$set":{"initializing":[fresh()]}},
                "initialization_in_progress",
            ),
            (
                doc! {"$unset":{"current.inventory_writer_cohort":""}},
                "inventory_unreviewed",
            ),
        ] {
            lifecycle
                .update_one(doc! {"_id":SINGLETON}, update)
                .await
                .unwrap();
            assert_eq!(
                reasons(&db, &f).await,
                BTreeSet::from([LOAD_BLOCKER, expected])
            );
            lifecycle
                .replace_one(doc! {"_id":SINGLETON}, &original)
                .await
                .unwrap();
        }
        let stored = f.config.hmac_keyset_path.take();
        assert_eq!(
            reasons(&db, &f).await,
            BTreeSet::from([LOAD_BLOCKER, "independent_hmac_unverified"])
        );
        f.config.hmac_keyset_path = stored;
        f.config.allow_legacy_record_reads = true;
        assert_eq!(
            reasons(&db, &f).await,
            BTreeSet::from([LOAD_BLOCKER, "reader_writer_policy"])
        );
        f.config.allow_legacy_record_reads = false;
        let previous = f.config.encryption_key_previous.take();
        assert_eq!(
            reasons(&db, &f).await,
            BTreeSet::from([LOAD_BLOCKER, "layout_mismatch"])
        );
        f.config.encryption_key_previous = previous;
        let replicas = db.collection::<Document>(crate::models::security_replica::COLLECTION_NAME);
        let id = doc! {"_id":"test-process"};
        let live = replicas.find_one(id.clone()).await.unwrap().unwrap();
        replicas
            .update_one(id.clone(), doc! {"$pull":{"capabilities":CAPABILITY}})
            .await
            .unwrap();
        assert_eq!(
            reasons(&db, &f).await,
            BTreeSet::from([LOAD_BLOCKER, "writer_fleet_unverified"])
        );
        replicas.replace_one(id, live).await.unwrap();
        let native = db.collection::<Document>("platform_entry_migrations");
        native
            .update_one(
                doc! {"_id":"native-entry-v2"},
                doc! {"$set":{"state":"preparing"}},
            )
            .await
            .unwrap();
        assert_eq!(
            reasons(&db, &f).await,
            BTreeSet::from([LOAD_BLOCKER, "native_parent_cleanup_unverified"])
        );
        native
            .update_one(
                doc! {"_id":"native-entry-v2"},
                doc! {"$set":{"state":"ready"}},
            )
            .await
            .unwrap();
        let migration = db.collection::<Document>("encrypted_record_migration_runs");
        migration
            .insert_one(doc! {"_id":"contextual-records-v1"})
            .await
            .unwrap();
        assert_eq!(
            reasons(&db, &f).await,
            BTreeSet::from([LOAD_BLOCKER, "migration_checkpoint_unreadable"])
        );
        migration
            .delete_one(doc! {"_id":"contextual-records-v1"})
            .await
            .unwrap();
        lifecycle
            .update_one(
                doc! {"_id":SINGLETON},
                doc! {"$set":{"restore_required":true}},
            )
            .await
            .unwrap();
        assert!(reasons(&db, &f).await.contains("restore_required"));
        assert!(
            step(
                &db,
                &f.config,
                &f.actor,
                &f.request,
                &CertifiedRelease::Rehearsal
            )
            .await
            .is_err()
        );
    })
    .await;
    db.drop().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the preserved 8b production executable"]
async fn old_binary_cannot_admit_the_survivor_before_or_after_publication() {
    let executable = std::env::var_os("NYXID_TEST_OLD_SERVER").unwrap();
    let db = connect_transaction_test_database("retire_old_binary").await;
    with_history(&db, 300, async {
        let mut f = fixture(&db).await;
        let mut url = url::Url::parse(&std::env::var("NYXID_TEST_DATABASE_URL").unwrap()).unwrap();
        let mut probe = |database: &Database, dual: bool, admitted: bool| {
            url.set_path(database.name());
            let mut process = tokio::process::Command::new(&executable);
            process
                .current_dir(f._files.path())
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap())
                .env("DATABASE_URL", url.as_str())
                .env("NYXID_BOOTSTRAP_DATABASE_URL", url.as_str())
                .env("NYXID_DEPLOYMENT_REVISION", "release-9")
                .env("CONTEXTUAL_RECORD_WRITERS_DRAINED", "true")
                .env("CONTEXTUAL_RECORD_WRITES", "true")
                .env("ALLOW_LEGACY_RECORD_READS", "false")
                .env("ENCRYPTION_KEY", f.config.encryption_key.as_ref().unwrap())
                .env(
                    "HMAC_KEYSET_PATH",
                    f.config.hmac_keyset_path.as_ref().unwrap(),
                )
                .args(["export-hmac-keyset", "--path", "exported"]);
            if dual {
                process.env(
                    "ENCRYPTION_KEY_PREVIOUS",
                    f.config.encryption_key_previous.as_ref().unwrap(),
                );
            }
            async move {
                let result = process.output().await.unwrap();
                if admitted {
                    assert!(
                        result.status.success(),
                        "old dual startup failed: {}",
                        String::from_utf8_lossy(&result.stderr)
                    );
                    return;
                }
                assert!(!result.status.success());
                let error = String::from_utf8_lossy(&result.stderr);
                assert!(
                    error.contains("Key lifecycle startup admission failed")
                        && error.contains("conflict"),
                    "unexpected startup stage: {error}"
                );
            }
        };
        probe(&db, false, false).await;
        for _ in 0..5 {
            f.request.expected_step_revision = step(
                &db,
                &f.config,
                &f.actor,
                &f.request,
                &CertifiedRelease::Rehearsal,
            )
            .await
            .unwrap()
            .revision;
        }
        probe(&db, false, false).await;
        probe(&db, true, false).await;
        assert!(!f._files.path().join("exported").exists());
        let control = connect_transaction_test_database("retire_old_control").await;
        let keyset = crate::crypto::hmac_lifecycle::HmacKeyset::load(std::path::Path::new(
            f.config.hmac_keyset_path.as_ref().unwrap(),
        ))
        .unwrap();
        hmac_lifecycle_service::ensure_consistent(&control, &keyset, Some(&keyset))
            .await
            .unwrap();
        enroll(
            &control,
            &f.config,
            &f.actor,
            &Enroll {
                recording: recording(0),
                deployment_revision: "release-9".into(),
            },
        )
        .await
        .unwrap();
        probe(&control, true, true).await;
        assert!(f._files.path().join("exported").exists());
        control.drop().await.unwrap();
    })
    .await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_plaintext_unresolved_values_and_old_envelopes_block_the_marker() {
    use crate::services::{durable_wake_tests::resume_after, saved_login_service as logins};
    let control = connect_transaction_test_database("retire_material_control").await;
    with_history(&control,300,async {
        for (case, reason) in [("plaintext","legacy_reader_required"),("malformed","unresolved_rows"),("previous","unresolved_rows")] {
            let db=connect_transaction_test_database("retire_material").await;
            let mut f=fixture(&db).await;
            let work=step(&db,&f.config,&f.actor,&f.request,&CertifiedRelease::Rehearsal);
            resume_after("retirement_claimed",work,async {
                match case {
                    "plaintext" => {
                        db.collection::<Document>("downstream_services").insert_one(doc!{"_id":fresh(),"created_by":&f.actor.user_id,"default_request_headers":[{"name":"x-reviewed","value":"plain"}]}).await.unwrap();
                    }
                    "malformed" => {
                        db.collection::<Document>("saved_logins").insert_one(doc!{"_id":fresh(),"user_id":&f.actor.user_id,"secret_context_version":3,"username_encrypted":true}).await.unwrap();
                    }
                    _ => {
                        let mut config=f.config.clone();
                        config.encryption_key=config.encryption_key_previous.take();
                        let input=logins::Input{label:"old".into(),allowed_origins:vec!["https://example.test".into()],username:"user".to_string().into(),password:Some("value".to_string().into()),totp_secret:None,confirm_each_sign_in:false};
                        logins::put(&db,&EncryptionKeys::from_config(&config),&f.actor.user_id,&f.actor.user_id,None,input).await.unwrap();
                    }
                }
            }).await;
            f.request.expected_step_revision+=1;
            let failure=step(&db,&f.config,&f.actor,&f.request,&CertifiedRelease::Rehearsal).await.err().unwrap();
            assert!(failure.to_string().contains(reason),"{case}: {failure}");
            assert!(!head(&db).await.contains_key("published_retirement"));
            db.drop().await.unwrap();
        }
    }).await;
    control.drop().await.unwrap();
}

#[tokio::test]
async fn dated_nonzero_retiring_bucket_blocks_an_otherwise_current_decision() {
    use crate::services::saved_login_service as logins;
    let db = connect_transaction_test_database("retire_bucket").await;
    with_history(&db, 300, async {
        let mut f = fixture(&db).await;
        let mut old = f.config.clone();
        old.encryption_key = old.encryption_key_previous.take();
        let input = logins::Input {
            label: "dated".into(),
            allowed_origins: vec!["https://example.test".into()],
            username: "fixture".to_string().into(),
            password: Some("fixture".to_string().into()),
            totp_secret: None,
            confirm_each_sign_in: false,
        };
        logins::put(
            &db,
            &EncryptionKeys::from_config(&old),
            &f.actor.user_id,
            &f.actor.user_id,
            None,
            input,
        )
        .await
        .unwrap();
        let keys = EncryptionKeys::from_config(&f.config);
        let id = report::start(&db, &keys, &f.config)
            .await
            .unwrap()
            .generation;
        let row = head(&db).await;
        let scope: Scope = field(&row, "scope").unwrap();
        let mut revision = row.get_i64("revision").unwrap();
        for (_, assertion) in guards::ASSERTIONS {
            revision = append(
                &db,
                &f.config,
                &f.actor,
                &Append {
                    recording: recording(revision),
                    scope: scope.clone(),
                    change: Change::Attest {
                        assertion,
                        affirmed: true,
                        valid_until_ms: None,
                    },
                },
            )
            .await
            .unwrap()
            .revision;
        }
        for _ in 0..128 {
            if report::advance(&db, &keys, &f.config, &id)
                .await
                .unwrap()
                .complete
            {
                break;
            }
        }
        f.request.target.generation = id;
        let result = evaluate(&db, &f.config, &f.request.target).await.unwrap();
        assert_eq!(
            result.blockers,
            BTreeSet::from([LOAD_BLOCKER, "non_survivor_key", "unresolved_rows"])
        );
        f.request.recording = recording(revision);
        f.request.expected_step_revision = revision;
        f.request.report_revision = result.report_revision.unwrap();
        assert!(
            step(
                &db,
                &f.config,
                &f.actor,
                &f.request,
                &CertifiedRelease::Rehearsal
            )
            .await
            .is_err()
        );
    })
    .await;
    db.drop().await.unwrap();
}
