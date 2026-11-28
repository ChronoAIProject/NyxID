use super::*;
use crate::services::{node_owner_service::ReplicaIdentity, saved_login_service as logins};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    process::{Child, Command},
    sync::RwLock,
    task::JoinHandle,
};

#[derive(Clone, serde::Deserialize)]
struct Endpoint {
    port: u16,
    token: String,
}

struct Peer {
    process: Child,
    endpoint: Endpoint,
}

struct Run<'a> {
    db: &'a Database,
    actor: &'a AuditActor,
    files: &'a Path,
    candidate: PathBuf,
    base: PathBuf,
}

impl Run<'_> {
    fn spawn(&self, binary: &Path, config: &AppConfig, ready: &Path) -> Child {
        let mut url = url::Url::parse(&std::env::var("NYXID_TEST_DATABASE_URL").unwrap()).unwrap();
        url.set_path(self.db.name());
        let mut cmd = Command::new(binary);
        cmd.args([
            "--exact",
            "services::key_lifecycle::rollout_peer::serve",
            "--ignored",
            "--nocapture",
        ])
        .env("NYXID_BOOTSTRAP_DATABASE_URL", url.as_str())
        .env("NYXID_DEPLOYMENT_REVISION", "release-9")
        .env("NYXID_REHEARSAL_OWNER", &self.actor.user_id)
        .env("NYXID_REHEARSAL_READY", ready)
        .env("ENCRYPTION_KEY", config.encryption_key.as_ref().unwrap())
        .env(
            "HMAC_KEYSET_PATH",
            config.hmac_keyset_path.as_ref().unwrap(),
        )
        .env_remove("ENCRYPTION_KEY_PREVIOUS")
        .kill_on_drop(true);
        if let Some(key) = &config.encryption_key_previous {
            cmd.env("ENCRYPTION_KEY_PREVIOUS", key);
        }
        cmd.spawn().unwrap()
    }
    async fn peer(&self, binary: &Path, config: &AppConfig) -> Peer {
        let ready = self.files.join(fresh());
        let mut process = self.spawn(binary, config, &ready);
        tokio::time::timeout(Duration::from_secs(120), async {
            while !ready.exists() {
                assert!(
                    process.try_wait().unwrap().is_none(),
                    "serving peer exited before admission"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        Peer {
            process,
            endpoint: serde_json::from_slice(&std::fs::read(ready).unwrap()).unwrap(),
        }
    }
    async fn refuses(&self, binary: &Path, config: &AppConfig) {
        let ready = self.files.join(fresh());
        let mut child = self.spawn(binary, config, &ready);
        let status = tokio::time::timeout(Duration::from_secs(40), child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(!status.success() && !ready.exists());
        assert_eq!(
            std::fs::read(ready.with_extension("refused")).unwrap(),
            b"ring_admission"
        );
    }
}

fn body() -> serde_json::Value {
    json!({"label":"rotation rehearsal","allowed_origins":["https://example.com"],"username":"fixture",
        "password":"rehearsal","confirm_each_sign_in":true})
}

struct Traffic {
    endpoint: Arc<RwLock<Endpoint>>,
    completed: Arc<AtomicUsize>,
    job: JoinHandle<()>,
}

impl Traffic {
    fn start(peer: &Peer, id: String) -> Self {
        let endpoint = Arc::new(RwLock::new(peer.endpoint.clone()));
        let completed = Arc::new(AtomicUsize::new(0));
        let target = endpoint.clone();
        let progress = completed.clone();
        let job = tokio::spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap();
            loop {
                {
                    let peer = target.read().await;
                    let url = format!("http://127.0.0.1:{}/api/v1/saved-logins", peer.port);
                    let read = client
                        .get(&url)
                        .bearer_auth(&peer.token)
                        .send()
                        .await
                        .unwrap();
                    assert_eq!(read.status().as_u16(), 200, "saved-login read");
                    let write = client
                        .put(format!("{url}/{id}"))
                        .bearer_auth(&peer.token)
                        .json(&body())
                        .send()
                        .await
                        .unwrap();
                    assert_eq!(write.status().as_u16(), 200, "saved-login replacement");
                    progress.fetch_add(1, Ordering::SeqCst);
                }
                tokio::time::sleep(Duration::from_millis(750)).await;
            }
        });
        Self {
            endpoint,
            completed,
            job,
        }
    }
    async fn mark(&self, phase: &str) {
        let before = self.completed.load(Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(15), async {
            while self.completed.load(Ordering::SeqCst) == before {
                assert!(!self.job.is_finished(), "traffic stopped during {phase}");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        println!(
            "ROLLOUT phase={phase} completed_read_write_pairs={}",
            self.completed.load(Ordering::SeqCst)
        );
    }
    async fn move_to(&self, peer: &Peer, phase: &str) {
        *self.endpoint.write().await = peer.endpoint.clone();
        self.mark(phase).await;
    }
}

fn runtime() -> fleet::Runtime {
    fleet::Runtime::new(
        ReplicaIdentity {
            instance_name: "coordinator".into(),
            generation_id: "test-coordinator".into(),
            internal_base_url: "http://127.0.0.1:1".into(),
        },
        fleet::RolloutConfig {
            expected_replicas: vec!["coordinator".into(), "traffic".into()],
            drained: true,
        },
        false,
    )
}

async fn heartbeat(db: &Database, config: &AppConfig) -> JoinHandle<()> {
    let ticket = join(db, config).await.unwrap();
    let init = Initialization::begin(db, Some(&ticket)).await.unwrap();
    let mut local = runtime();
    local.retirement_cohort = ticket.cohort;
    fleet::observe_with_automation(db, &local, false)
        .await
        .unwrap();
    init.finish(db).await.unwrap();
    let db = db.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            fleet::observe_with_automation(&db, &local, false)
                .await
                .unwrap();
        }
    })
}

async fn wait_for_fleet(db: &Database) {
    let cohort = layout_cohort(&head(db).await).await.unwrap();
    tokio::time::timeout(Duration::from_secs(100), async {
        while guards::fleet_deadline(db, &cohort).await.is_err() {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })
    .await
    .expect("previous generation leaves the readiness horizon");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the PR 9 executable with the loopback adapter"]
async fn mixed_release_serves_through_rotation_and_second_prepare() {
    let db = connect_transaction_test_database("rkroll").await;
    with_history(&db, 300, rehearse(&db)).await;
    db.drop().await.unwrap();
}

async fn rehearse(db: &Database) {
    let (ab, actor, files, _) = installation(db).await;
    let run = Run {
        db,
        actor: &actor,
        files: files.path(),
        candidate: std::env::current_exe().unwrap(),
        base: std::env::var_os("NYXID_REHEARSAL_BASE").unwrap().into(),
    };
    fleet::initialize(runtime());
    let mut heart = heartbeat(db, &ab).await;
    let initial = status(db, &ab).await.unwrap().unwrap();
    let a = initial.keys[0].key_ref.clone();
    let b = initial.keys[1].key_ref.clone();
    let mut ba = ab.clone();
    std::mem::swap(&mut ba.encryption_key, &mut ba.encryption_key_previous);
    let mut only = ba.clone();
    let retired = only.encryption_key_previous.take().unwrap();
    let login = logins::put(
        db,
        &EncryptionKeys::from_config(&ab),
        &actor.user_id,
        &actor.user_id,
        None,
        serde_json::from_value(body()).unwrap(),
    )
    .await
    .unwrap();

    let mut older = run.peer(&run.base, &ab).await;
    let traffic = Traffic::start(&older, login.id.clone());
    traffic.mark("base_a_writes").await;
    let mut original = run.peer(&run.candidate, &ab).await;
    traffic.move_to(&original, "candidate_a_writes").await;
    assert!(edit(db, &ab, &actor, Operation::BeginRoll).await.is_err());
    older.process.kill().await.unwrap();
    wait_for_fleet(db).await;
    edit(db, &ab, &actor, Operation::BeginRoll).await.unwrap();
    let mut reverse = run.peer(&run.candidate, &ba).await;
    traffic.move_to(&reverse, "first_b_writes").await;
    edit(db, &ab, &actor, Operation::SelectWriter { primary: a })
        .await
        .unwrap();
    traffic.move_to(&original, "interrupted_rollback_a").await;
    reverse.process.kill().await.unwrap();
    run.refuses(&run.candidate, &ba).await;
    wait_for_fleet(db).await;
    edit(db, &ab, &actor, Operation::BeginRoll).await.unwrap();
    let mut dual = run.peer(&run.candidate, &ba).await;
    traffic.move_to(&dual, "forward_b_writes").await;
    edit(db, &ba, &actor, Operation::SelectWriter { primary: b })
        .await
        .unwrap();
    original.process.kill().await.unwrap();
    heart.abort();
    heart = heartbeat(db, &ba).await;
    wait_for_fleet(db).await;
    let row = head(db).await;
    let receipt = Receipt {
        entry_id: fresh(),
        revision: field(&row, "revision").unwrap(),
        scope: field(&row, "scope").unwrap(),
        recorded_at_ms: 0,
    };
    let mut target = clean_census(db, &ba, &actor, receipt).await;
    assert!(
        advance(db, &ba, &actor, &target)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains(LOAD_BLOCKER)
    );
    for label in [
        "decision",
        "approval",
        "consumption_marker",
        "consumption_receipt",
        "publication",
    ] {
        let before = traffic.completed.load(Ordering::SeqCst);
        let transition = step(db, &ba, &actor, &target, &CertifiedRelease::Rehearsal);
        let (progress, ()) = tokio::join!(transition, traffic.mark(label));
        assert!(traffic.completed.load(Ordering::SeqCst) > before);
        target.expected_step_revision = progress.unwrap().revision;
    }
    let mut survivor = run.peer(&run.candidate, &only).await;
    traffic.move_to(&survivor, "survivor_only").await;
    dual.process.kill().await.unwrap();
    heart.abort();
    let heart = heartbeat(db, &only).await;
    run.refuses(&run.base, &ab).await;
    run.refuses(&run.candidate, &ba).await;
    assert!(
        edit(
            db,
            &only,
            &actor,
            Operation::Prepare {
                proposal_id: fresh()
            }
        )
        .await
        .is_err()
    );
    wait_for_fleet(db).await;
    edit(
        db,
        &only,
        &actor,
        Operation::Close {
            generation: target.target.generation,
        },
    )
    .await
    .unwrap();
    let bc = prepare_after_close(db, &only, &actor, &retired, files.path()).await;
    let mut next = run.peer(&run.candidate, &bc).await;
    traffic.move_to(&next, "c_prepared_b_writes").await;
    let retained = logins::get(db, &actor.user_id, &login.id).await.unwrap();
    logins::verify_current_secrets(&EncryptionKeys::from_config(&only), &retained)
        .await
        .unwrap();
    traffic.job.abort();
    heart.abort();
    survivor.process.kill().await.unwrap();
    next.process.kill().await.unwrap();
}
