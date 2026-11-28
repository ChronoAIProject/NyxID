//! The worker always creates a fresh test database. The parent kills the
//! worker at an acknowledged durable boundary and inspects it from a new client.
use super::*;
use crate::services::durable_wake_tests::resume_after;
use std::{path::PathBuf, time::Duration};
use tokio::process::Command;

const WORKER: &str = "services::key_lifecycle::retirement::tests::crash::coordinator_child";

#[tokio::test]
#[ignore = "driven by the process-death tests"]
async fn coordinator_child() {
    let directory = PathBuf::from(std::env::var_os("NYXID_RETIREMENT_CRASH_FIXTURE").unwrap());
    let site = match std::env::var("NYXID_RETIREMENT_BOUNDARY").unwrap().as_str() {
        "claim" => "retirement_claimed",
        "commit" => "retirement_committed",
        "capture" => "capture_claimed",
        "witness" => "capture_checkpoint",
        _ => panic!("unexpected boundary"),
    };
    let stage = std::env::var("NYXID_RETIREMENT_STAGE")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let db = connect_transaction_test_database("rk_crash").await;
    let mut f = fixture(&db).await;
    for _ in 0..stage {
        let result = step(&db, &f.config, &f.actor, &f.request, &CertifiedRelease::Rehearsal)
            .await
            .unwrap();
        f.request.expected_step_revision = result.revision;
    }
    let info = json!({"database":db.name(),"actor":f.actor.user_id,"hmac":f.config.hmac_keyset_path,"request":&f.request});
    std::fs::write(
        directory.join("recovery.json"),
        serde_json::to_vec(&info).unwrap(),
    )
    .unwrap();
    resume_after(
        site,
        step(&db, &f.config, &f.actor, &f.request, &CertifiedRelease::Rehearsal),
        async {
            std::fs::write(directory.join("boundary"), site).unwrap();
            std::future::pending::<()>().await;
        },
    )
    .await;
}

async fn crash_cases(boundary: &str, first: usize) {
    let control = connect_transaction_test_database("rk_parent").await;
    with_history(&control,300,async {
        for stage in first..5 {
            let dir=tempfile::tempdir().unwrap();
            let mut child=Command::new(std::env::current_exe().unwrap())
                .args(["--exact",WORKER,"--ignored","--nocapture"])
                .env("NYXID_RETIREMENT_CRASH_FIXTURE",dir.path())
                .env("NYXID_RETIREMENT_BOUNDARY",boundary)
                .env("NYXID_RETIREMENT_STAGE",stage.to_string()).kill_on_drop(true).spawn().unwrap();
            tokio::time::timeout(Duration::from_secs(75),async {
                while !dir.path().join("boundary").exists() {
                    assert!(child.try_wait().unwrap().is_none(),"worker exited before {boundary} in stage {stage}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }).await.unwrap();
            child.kill().await.unwrap();
            child.wait().await.unwrap();
            let info:serde_json::Value=serde_json::from_slice(&std::fs::read(dir.path().join("recovery.json")).unwrap()).unwrap();
            let name=info["database"].as_str().unwrap();
            assert!(name.starts_with("nyxid_test_") && name.ends_with("_rk_crash"));
            let db=control.client().database(name);
            let actor=AuditActor::from_auth_user(&test_auth_user(info["actor"].as_str().unwrap()));
            let mut req:Request=serde_json::from_value(info["request"].clone()).unwrap();
            let mut config=test_app_config();
            config.encryption_key_previous=Some("5a".repeat(32));
            config.contextual_record_writes=true;
            config.allow_legacy_record_reads=false;
            config.hmac_keyset_path=Some(info["hmac"].as_str().unwrap().into());
            let row=head(&db).await;
            let committed=boundary=="commit";
            assert_eq!(row.get_i64("revision"),Ok(req.expected_step_revision+i64::from(committed)));
            if boundary=="capture" || boundary=="claim" {
                assert!(step(&db,&config,&actor,&req,&CertifiedRelease::Rehearsal).await.is_err());
                assert!(!row.contains_key("published_retirement"));
            }else{
                fleet::observe_retirement_test_replica(&db,layout_cohort(&row).await.unwrap()).await;
                for _ in 0..6 {
                    let status=step(&db,&config,&actor,&req,&CertifiedRelease::Rehearsal).await.unwrap();
                    if status.phase==Phase::Published {break;}
                    req.expected_step_revision=status.revision;
                }
                let final_head=head(&db).await;
                config.encryption_key_previous=None;
                let keys=admission_reader(&config,&final_head).await.unwrap();
                assert!(publication(&db,&keys,&final_head).await.is_ok());
                assert_eq!(db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                    .count_documents(doc!{"event_type":"encryption_key_lifecycle.retirement","event_data.phase":"published"}).await.unwrap(),1);
            }
            db.drop().await.unwrap();
        }
    }).await;
    control.drop().await.unwrap();
}

#[tokio::test]
async fn kill_after_transition_claims() {
    crash_cases("claim", 0).await;
}
#[tokio::test]
async fn kill_after_transition_commits() {
    crash_cases("commit", 0).await;
}
#[tokio::test]
async fn kill_after_capture_claims() {
    crash_cases("capture", 1).await;
}
#[tokio::test]
async fn kill_after_witness_checkpoints() {
    crash_cases("witness", 1).await;
}
