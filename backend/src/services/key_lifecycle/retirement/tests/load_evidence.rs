use super::*;
use crate::services::key_lifecycle::load_gate::*;

async fn append_change(db: &Database, f: &Fixture, change: Change) -> AppResult<Receipt> {
    let row = head(db).await;
    append(
        db,
        &f.config,
        &f.actor,
        &Append {
            recording: recording(field(&row, "revision")?),
            scope: field(&row, "scope")?,
            change,
        },
    )
    .await
}

#[tokio::test]
async fn only_scoped_predeclared_isolated_measurements_can_authorize_a_decision() {
    let db = connect_transaction_test_database("isolated_evidence").await;
    with_history(&db, 300, check(&db)).await;
    db.drop().await.unwrap();
}

async fn check(db: &Database) {
    let mut f = fixture(db).await;
    let report = report::status(
        db,
        &EncryptionKeys::from_config(&f.config),
        &f.request.target.generation,
    )
    .await
    .unwrap();
    let digest = serde_json::to_value(report).unwrap()["registry_digest"]
        .as_str()
        .unwrap()
        .to_owned();
    let plan = Protocol {
        schema_version: 1,
        registry_digest: digest,
        baseline_sha256: "1".repeat(64),
        candidate_sha256: executable().await.unwrap().into(),
        workload_sha256: "3".repeat(64),
        rig_ref: fresh(),
        slo_ref: fresh(),
        offered_rps: 100_000,
        minimum_rps: 99_000,
        operations_per_arm: 400,
        max_p95_us: 1000,
        max_p99_us: 2000,
        confidence_bps: 9500,
        margin_bps: 100,
        paired_rounds: 8,
    };
    let mut relaxed = plan.clone();
    relaxed.margin_bps += 1;
    assert!(
        append_change(db, &f, Change::DeclareLoadProtocol { protocol: relaxed })
            .await
            .is_err()
    );
    let declaration = append_change(
        db,
        &f,
        Change::DeclareLoadProtocol {
            protocol: plan.clone(),
        },
    )
    .await
    .unwrap();
    let sample = Sample {
        posted: 400,
        posting_span_us: 3990,
        completed: 400,
        errors: 0,
        elapsed_us: 4000,
        p95_us: 500,
        p99_us: 800,
        source_commands: 1200,
        foreground_census_commands: 0,
        kms_calls: 0,
    };
    let block = Round {
        base: sample.clone(),
        base_repeat: sample.clone(),
        candidate_idle: sample.clone(),
        candidate_capture: sample.clone(),
        candidate_publication: sample,
        authenticated_cuts: 1,
        writes_during_publication: 3,
    };
    let result = Measurement {
        schema_version: 1,
        protocol_id: declaration.entry_id,
        environment: Environment::Isolated,
        rig_ref: plan.rig_ref.clone(),
        workload_sha256: plan.workload_sha256.clone(),
        baseline_sha256: plan.baseline_sha256.clone(),
        candidate_sha256: plan.candidate_sha256.clone(),
        offered_rps: 100_000,
        started_ms: declaration.recorded_at_ms + 1,
        finished_ms: declaration.recorded_at_ms + 201,
        rounds: vec![block; 8],
    };
    tokio::time::sleep(std::time::Duration::from_millis(205)).await;
    let mut wrong = result.clone();
    wrong.rig_ref = fresh();
    assert!(
        append_change(db, &f, Change::RecordLoadResult { result: wrong })
            .await
            .is_err()
    );
    for kind in 0..9 {
        let mut bad = result.clone();
        match kind {
            0 => bad.environment = Environment::Shared,
            1 => {
                for round in &mut bad.rounds {
                    round.base_repeat.p95_us += 15;
                }
            }
            2 => bad.rounds[0].candidate_publication.errors = 1,
            3 => bad.rounds[0].candidate_capture.p99_us = plan.max_p99_us + 1,
            4 => bad.rounds[0].candidate_idle.elapsed_us *= 2,
            5 => {
                bad.rounds[0]
                    .candidate_publication
                    .foreground_census_commands = 1
            }
            6 => bad.rounds[0].writes_during_publication = 0,
            7 => bad.rounds[0].candidate_publication.posting_span_us /= 2,
            _ => bad.rounds[0].base_repeat.posted -= 1,
        }
        let receipt = append_change(db, &f, Change::RecordLoadResult { result: bad })
            .await
            .unwrap();
        f.request.recording.expected_revision = receipt.revision;
        f.request.expected_step_revision = receipt.revision;
        assert!(
            advance(db, &f.config, &f.actor, &f.request)
                .await
                .err()
                .unwrap()
                .to_string()
                .contains(LOAD_BLOCKER)
        );
        assert!(!head(db).await.contains_key("retirement_id"));
    }
    let receipt = append_change(db, &f, Change::RecordLoadResult { result })
        .await
        .unwrap();
    f.request.recording.expected_revision = receipt.revision;
    f.request.expected_step_revision = receipt.revision;
    let evaluation = evaluate(db, &f.config, &f.request.target).await.unwrap();
    assert!(evaluation.blockers.is_empty() && evaluation.certification_enabled);
    assert_eq!(
        advance(db, &f.config, &f.actor, &f.request)
            .await
            .unwrap()
            .phase,
        Phase::Decision
    );
    append_change(db, &f, Change::DeclareLoadProtocol { protocol: plan })
        .await
        .unwrap();
    assert!(
        advance(db, &f.config, &f.actor, &f.request)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains(LOAD_BLOCKER)
    );
}

#[tokio::test]
async fn credential_dependency_withdraws_the_catalog_session_assertion_in_the_same_revision() {
    let db = connect_transaction_test_database("ddl_scope").await;
    with_history(&db, 300, async {
        let f = fixture(&db).await;
        let opened = append_change(
            &db,
            &f,
            Change::OpenDependency {
                inventory: Inventory::DatabaseCredential,
                parents: Default::default(),
                formats: ["catalog".into()].into(),
            },
        )
        .await
        .unwrap();
        let checks = evaluate(&db, &f.config, &f.request.target).await.unwrap();
        assert!(
            checks.blockers.contains("catalog_access_withdrawn")
                && checks.blockers.contains("dependency_open")
        );
        let attest = || Change::Attest {
            assertion: Assertion::CatalogAccessWithdrawn,
            affirmed: true,
            valid_until_ms: None,
        };
        assert!(append_change(&db, &f, attest()).await.is_err());
        append_change(
            &db,
            &f,
            Change::CloseDependency {
                dependency_id: opened.entry_id,
                resolution: Resolution::RevokedOrRotated,
            },
        )
        .await
        .unwrap();
        assert!(
            evaluate(&db, &f.config, &f.request.target)
                .await
                .unwrap()
                .blockers
                .contains("catalog_access_withdrawn")
        );
        append_change(
            &db,
            &f,
            Change::ReviewInventory {
                inventory: Inventory::DatabaseCredential,
                disposition: Review::ReviewedEmpty,
            },
        )
        .await
        .unwrap();
        append_change(&db, &f, attest()).await.unwrap();
        assert!(
            !evaluate(&db, &f.config, &f.request.target)
                .await
                .unwrap()
                .blockers
                .contains("catalog_access_withdrawn")
        );
    })
    .await;
    db.drop().await.unwrap();
}
