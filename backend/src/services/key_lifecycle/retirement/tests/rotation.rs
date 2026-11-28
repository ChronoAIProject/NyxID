use super::*;
use crate::services::key_lifecycle::rotation::{Operation, Request as LayoutRequest, submit_at};
use std::{io::Write, path::Path};

async fn request(db: &Database, transition: Operation) -> LayoutRequest {
    let head = head(db).await;
    LayoutRequest {
        recording: recording(field(&head, "revision").unwrap()),
        scope: field(&head, "scope").unwrap(),
        transition,
    }
}

async fn edit(
    db: &Database,
    config: &AppConfig,
    actor: &AuditActor,
    operation: Operation,
) -> AppResult<Receipt> {
    submit_at(db, config, actor, &request(db, operation).await, None).await
}

async fn join(db: &Database, config: &AppConfig) -> AppResult<Admission> {
    let row = head(db).await;
    admit_enrolled(db, db, config, field(&row, "revision")?, "release-9").await
}

fn proposal(directory: &Path, id: &str, value: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new_in(directory).unwrap();
    serde_json::to_writer(
        file.as_file_mut(),
        &json!({"proposal_id":id,"provider":"local","previous":value}),
    )
    .unwrap();
    file.flush().unwrap();
    file
}

async fn prepare_after_close(
    db: &Database,
    survivor: &AppConfig,
    actor: &AuditActor,
    retired: &str,
    dir: &Path,
) -> AppConfig {
    let id = fresh();
    let valid_value = "36".repeat(32);
    let good = proposal(dir, &id, &valid_value);
    let old = proposal(dir, &id, retired);
    let req = || Operation::Prepare {
        proposal_id: id.clone(),
    };
    let before = head(db).await;
    let invalid = request(db, req()).await;
    let left = request(db, req()).await;
    let right = request(db, req()).await;
    let (first, second, bad) = tokio::join!(
        submit_at(db, survivor, actor, &left, Some(good.path())),
        submit_at(db, survivor, actor, &right, Some(good.path())),
        submit_at(db, survivor, actor, &invalid, Some(old.path()))
    );
    assert!(bad.is_err());
    assert_ne!(first.is_ok(), second.is_ok());
    let winner = if first.is_ok() { &left } else { &right };
    let replay = submit_at(db, survivor, actor, winner, Some(good.path()))
        .await
        .unwrap();
    assert_eq!(replay.revision, before.get_i64("revision").unwrap() + 1);
    assert_eq!(head(db).await.get_i64("revision"), Ok(replay.revision));
    assert!(join(db, survivor).await.is_err());
    let mut bc = survivor.clone();
    bc.encryption_key_previous = Some(valid_value);
    join(db, &bc).await.unwrap();
    let mut cb = bc.clone();
    std::mem::swap(&mut cb.encryption_key, &mut cb.encryption_key_previous);
    assert!(join(db, &cb).await.is_err());
    bc
}

#[tokio::test]
async fn published_scope_stays_open_until_all_survivor_restarts_are_accounted_for() {
    let db = connect_transaction_test_database("rotation_close").await;
    with_history(&db, 300, async {
        let (config, actor, files, initial) = installation(&db).await;
        let dependency = append(
            &db,
            &config,
            &actor,
            &Append {
                recording: recording(initial.revision),
                scope: initial.scope.clone(),
                change: Change::OpenDependency {
                    inventory: Inventory::Rollback,
                    parents: Default::default(),
                    formats: ["contextual".into()].into(),
                },
            },
        )
        .await
        .unwrap();
        let resolve = || Change::CloseDependency {
            dependency_id: dependency.entry_id.clone(),
            resolution: Resolution::TestedSurvivorRollback,
        };
        let receipt = append(
            &db,
            &config,
            &actor,
            &Append {
                recording: recording(dependency.revision),
                scope: dependency.scope.clone(),
                change: resolve(),
            },
        )
        .await
        .unwrap();
        fleet::observe_retirement_test_replica(&db, layout_cohort(&head(&db).await).await.unwrap())
            .await;
        let request = clean_census(&db, &config, &actor, receipt).await;
        let mut f = Fixture {
            config,
            actor,
            _files: files,
            request,
        };
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
        let mut survivor = f.config.clone();
        let retired = survivor.encryption_key_previous.take().unwrap();
        let state = status(&db, &survivor).await.unwrap().unwrap();
        assert!(state.rotation_open);
        let close = || Operation::Close {
            generation: f.request.target.generation.clone(),
        };
        assert!(
            edit(
                &db,
                &survivor,
                &f.actor,
                Operation::Prepare {
                    proposal_id: fresh()
                }
            )
            .await
            .is_err()
        );
        assert!(edit(&db, &survivor, &f.actor, close()).await.is_err());
        let admission = join(&db, &survivor).await.unwrap();
        fleet::observe_retirement_test_replica(&db, admission.cohort.unwrap()).await;
        edit(&db, &survivor, &f.actor, close()).await.unwrap();
        assert!(!status(&db, &survivor).await.unwrap().unwrap().rotation_open);
        let bc = prepare_after_close(&db, &survivor, &f.actor, &retired, f._files.path()).await;
        assert!(status(&db, &bc).await.unwrap().unwrap().rotation_open);
        assert!(join(&db, &f.config).await.is_err());
        let current = status(&db, &bc).await.unwrap().unwrap();
        let mut renew = Append {
            recording: recording(current.revision),
            scope: current.scope,
            change: resolve(),
        };
        assert!(append(&db, &bc, &f.actor, &renew).await.is_err());
        renew.scope.keys = state.scope.keys;
        let renewed = append(&db, &bc, &f.actor, &renew).await.unwrap();
        let stored = db
            .collection::<Dependency>(DEPENDENCIES)
            .find_one(doc! {"_id":&dependency.entry_id})
            .await
            .unwrap()
            .unwrap();
        assert!(stored.opened.scope == dependency.scope);
        assert!(stored.last_record.scope == renewed.scope);
    })
    .await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn rollback_and_stale_initializers_cannot_admit_an_unreviewed_layout() {
    let db = connect_transaction_test_database("rotation_roll").await;
    let (ab, actor, _files, _) = installation(&db).await;
    let ticket = join(&db, &ab).await.unwrap();
    fleet::observe_retirement_test_replica(&db, ticket.cohort.clone().unwrap()).await;
    let mut ba = ab.clone();
    std::mem::swap(&mut ba.encryption_key, &mut ba.encryption_key_previous);
    let original = status(&db, &ab).await.unwrap().unwrap();
    let a = original.keys[0].key_ref.clone();
    let b = original.keys[1].key_ref.clone();
    assert!(join(&db, &ba).await.is_err());
    edit(&db, &ab, &actor, Operation::BeginRoll).await.unwrap();
    let paused = join(&db, &ba).await.unwrap();
    assert!(Initialization::begin(&db, Some(&ticket)).await.is_err());
    let mut wrong = ab.clone();
    wrong.encryption_key_previous = Some("36".repeat(32));
    assert!(join(&db, &wrong).await.is_err());
    wrong.encryption_key = Some("36".repeat(32));
    wrong.encryption_key_previous = ba.encryption_key.clone();
    assert!(join(&db, &wrong).await.is_err());
    wrong = ab.clone();
    wrong.key_provider = "unrecognized".into();
    assert!(join(&db, &wrong).await.is_err());
    assert!(
        edit(
            &db,
            &ba,
            &actor,
            Operation::Prepare {
                proposal_id: fresh()
            }
        )
        .await
        .is_err()
    );
    edit(&db, &ab, &actor, Operation::SelectWriter { primary: a })
        .await
        .unwrap();
    assert!(Initialization::begin(&db, Some(&paused)).await.is_err());
    assert!(join(&db, &ba).await.is_err());
    edit(&db, &ab, &actor, Operation::BeginRoll).await.unwrap();
    edit(&db, &ba, &actor, Operation::SelectWriter { primary: b })
        .await
        .unwrap();
    assert!(join(&db, &ab).await.is_err());
    let paused = join(&db, &ba).await.unwrap();
    let restored = request(&db, Operation::BeginRoll).await;
    append(
        &db,
        &ba,
        &actor,
        &Append {
            recording: restored.recording.clone(),
            scope: restored.scope.clone(),
            change: Change::AdvanceRestoreEpoch,
        },
    )
    .await
    .unwrap();
    assert!(Initialization::begin(&db, Some(&paused)).await.is_err());
    assert!(submit_at(&db, &ba, &actor, &restored, None).await.is_err());
    let clone = connect_transaction_test_database("rotation_restore").await;
    clone
        .collection::<Document>(HEAD)
        .insert_one(head(&db).await)
        .await
        .unwrap();
    assert!(join(&clone, &ba).await.is_err());
    let replay = request(&clone, Operation::BeginRoll).await;
    assert!(submit_at(&clone, &ba, &actor, &replay, None).await.is_err());
    assert_eq!(head(&clone).await.get_bool("restore_required"), Ok(true));
    clone.drop().await.unwrap();
    db.drop().await.unwrap();
}

#[cfg(unix)]
mod rehearsal;
