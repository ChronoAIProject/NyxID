use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::{Document, doc};
use std::{
    os::unix::process::ExitStatusExt,
    process::Stdio,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

static BOUND: AtomicBool = AtomicBool::new(false);

pub(super) fn listeners_bound() {
    BOUND.store(true, Ordering::SeqCst);
    println!("STARTUP_LISTENERS_BOUND");
}

pub(super) fn received_signal(signal: &str, received_at: DateTime<Utc>, ordinal: usize) {
    if std::env::var_os("NYXID_TEST_STARTUP_STEP").is_some() {
        println!(
            "STARTUP_SIGNAL {}",
            serde_json::json!({"signal": signal, "received_at": received_at, "ordinal": ordinal})
        );
    }
}

pub(super) async fn at_step(step: &str) {
    if std::env::var("NYXID_TEST_STARTUP_STEP").as_deref() != Ok(step) {
        return;
    }
    println!(
        "STARTUP_HOOK {step} bound={} workers={}",
        BOUND.load(Ordering::SeqCst),
        crate::services::shutdown::process().tracked_units()
    );
    // The parent delivers and observes the signal before allowing this step to
    // complete. No startup future may be dropped while this read is pending.
    let mut byte = [0];
    tokio::io::stdin().read_exact(&mut byte).await.unwrap();
    assert_eq!(byte, [b'R']);
    println!("STARTUP_STEP_COMPLETED {step}");
}

#[test]
fn startup_signal_child() {
    if std::env::var_os("NYXID_TEST_STARTUP_STEP").is_none() {
        return;
    }
    let cli = crate::Cli {
        promote_admin: None,
        command: std::env::var_os("NYXID_TEST_STARTUP_ONE_SHOT").map(|_| {
            crate::Commands::CleanupOrphans(crate::cleanup_cli::CleanupArgs {
                dry_run: true,
                yes: false,
            })
        }),
    };
    if cli.command.is_some() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(crate::server_run(cli));
        let workers = crate::services::shutdown::process().tracked_units();
        println!("ONE_SHOT_FINISHED workers={workers}");
        assert_eq!(workers, 0, "one-shot command started background workers");
    } else {
        crate::server_main(cli);
    }
}

async fn read_until(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    transcript: &mut String,
    prefix: &str,
) -> Option<String> {
    while let Some(line) = lines.next_line().await.unwrap() {
        transcript.push_str(&line);
        transcript.push('\n');
        if line.starts_with(prefix) {
            return Some(line);
        }
    }
    None
}

async fn check_step(step: &str, second: bool, one_shot: bool) {
    // This database starts completely empty so the early safe points prove
    // that even schema/seed/readiness writes have not happened.
    let database_url = std::env::var("NYXID_TEST_DATABASE_URL").unwrap();
    let client = mongodb::Client::with_uri_str(&database_url).await.unwrap();
    let db = client.database(&format!(
        "nyxid_test_startup_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let directory = tempfile::tempdir().unwrap();
    let key = directory.path().join("private.pem");
    let public = directory.path().join("public.pem");
    let generated = tokio::process::Command::new("openssl")
        .args([
            "genpkey",
            "-algorithm",
            "RSA",
            "-pkeyopt",
            "rsa_keygen_bits:4096",
            "-out",
        ])
        .arg(&key)
        .output()
        .await
        .unwrap();
    assert!(generated.status.success());
    let generated = tokio::process::Command::new("openssl")
        .args(["pkey", "-in"])
        .arg(&key)
        .args(["-pubout", "-out"])
        .arg(&public)
        .output()
        .await
        .unwrap();
    assert!(generated.status.success());
    let mut url = url::Url::parse(&database_url).unwrap();
    url.set_path(db.name());
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "startup_signal_tests::startup_signal_child",
            "--nocapture",
        ])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("RUST_MIN_STACK", "8388608")
        .env("NYXID_TEST_STARTUP_STEP", step)
        .env("DATABASE_URL", url.as_str())
        .env("ENCRYPTION_KEY", "11".repeat(32))
        .env("JWT_PRIVATE_KEY_PATH", &key)
        .env("JWT_PUBLIC_KEY_PATH", &public)
        .env("SHARE_ANALYTICS", "false")
        // Keep budget expiry out of hook-driven tests; deadline arithmetic is
        // covered separately with a controlled clock. The watchdog is shorter.
        .env("SHUTDOWN_DRAIN_TIMEOUT_SECS", "3600")
        .env("CHAIN_VERIFY_INTERVAL_SECS", "3600")
        .env("PORT", "0")
        .env("INTERNAL_BIND_ADDR", "127.0.0.1:0")
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env("ALL_PROXY", "http://127.0.0.1:1")
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env("RUST_LOG", "error")
        .current_dir(directory.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if one_shot {
        command.env("NYXID_TEST_STARTUP_ONE_SHOT", "1");
    }
    let mut child = command.spawn().unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut stderr = child.stderr.take().unwrap();
    let mut transcript = String::new();
    let mut received_at = None;
    let execution = async {
        if step != "one_shot" {
            assert!(
                read_until(
                    &mut lines,
                    &mut transcript,
                    &format!("STARTUP_HOOK {step} ")
                )
                .await
                .is_some(),
                "{transcript}"
            );
            assert_eq!(
                unsafe { libc::kill(child.id().unwrap() as libc::pid_t, libc::SIGTERM) },
                0
            );
            if step != "serving" && !one_shot {
                // EOF is the unfixed server dying under SIGTERM. Preserve its
                // actual exit status for the regression assertion below.
                if let Some(line) = read_until(&mut lines, &mut transcript, "STARTUP_SIGNAL ").await
                {
                    let event: serde_json::Value =
                        serde_json::from_str(line.strip_prefix("STARTUP_SIGNAL ").unwrap())
                            .unwrap();
                    assert_eq!(event["ordinal"], 1);
                    received_at = Some(event["received_at"].as_str().unwrap().to_owned());
                    assert!(
                        child.try_wait().unwrap().is_none(),
                        "startup step was cancelled"
                    );
                    if second {
                        assert_eq!(
                            unsafe { libc::kill(child.id().unwrap() as libc::pid_t, libc::SIGINT) },
                            0
                        );
                        let line = read_until(&mut lines, &mut transcript, "STARTUP_SIGNAL ")
                            .await
                            .expect("second signal not captured");
                        assert!(line.contains("\"ordinal\":2"), "{line}");
                        assert!(
                            child.try_wait().unwrap().is_none(),
                            "second signal cancelled a startup step"
                        );
                    }
                    input.write_all(b"R").await.unwrap();
                }
            } else if !one_shot {
                input.write_all(b"R").await.unwrap();
            }
        }
        drop(input);
        read_until(&mut lines, &mut transcript, "NEVER_A_PROTOCOL_LINE").await;
        let status = child.wait().await.unwrap();
        let mut error_output = String::new();
        stderr.read_to_string(&mut error_output).await.unwrap();
        transcript.push_str(&error_output);
        status
    };
    // A deadlock watchdog, not a speed assertion. The parent, not a sleep,
    // decides when each startup step may complete.
    let result = tokio::time::timeout(Duration::from_secs(180), execution).await;
    let rows: Vec<Document> = db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {"event_type":{"$in":["backend_process_started", "backend_shutdown_drain_started", "backend_shutdown_drain"]}})
        .await.unwrap().try_collect().await.unwrap();
    let collections = db.list_collection_names().await.unwrap();
    db.drop().await.unwrap();
    let status = result.unwrap_or_else(|_| panic!("startup child stuck: {transcript}"));
    if one_shot && step == "entry" {
        assert_eq!(status.signal(), Some(libc::SIGTERM), "{transcript}");
        assert!(collections.is_empty());
        return;
    }
    assert_eq!(status.code(), Some(0), "{step}: {transcript}");
    let count = |event| {
        rows.iter()
            .filter(|row| row.get_str("event_type") == Ok(event))
            .count()
    };
    if step == "one_shot" {
        assert!(
            transcript.contains("ONE_SHOT_FINISHED workers=0"),
            "{transcript}"
        );
        assert!(rows.is_empty());
    } else {
        assert!(
            transcript.contains(&format!("STARTUP_STEP_COMPLETED {step}")),
            "{transcript}"
        );
        assert_eq!(
            transcript.contains("STARTUP_LISTENERS_BOUND"),
            step == "serving",
            "{transcript}"
        );
        if step == "after_workers" {
            assert!(
                !transcript.contains("STARTUP_HOOK after_workers bound=false workers=0"),
                "{transcript}"
            );
        }
        if matches!(step, "entry" | "database_ready") {
            assert!(
                transcript.contains(&format!("STARTUP_HOOK {step} bound=false workers=0")),
                "{transcript}"
            );
            assert!(
                collections.is_empty(),
                "early stop wrote collections: {collections:?}"
            );
        } else {
            assert_eq!(
                count("backend_process_started"),
                usize::from(step == "serving")
            );
            assert_eq!(count("backend_shutdown_drain_started"), 1, "{transcript}");
            assert_eq!(count("backend_shutdown_drain"), 1, "{transcript}");
            let finish = rows
                .iter()
                .find(|row| row.get_str("event_type") == Ok("backend_shutdown_drain"))
                .unwrap()
                .get_document("event_data")
                .unwrap();
            assert_eq!(
                finish.get_str("end").unwrap(),
                if second { "second_signal" } else { "idle" }
            );
            if let Some(received_at) = received_at {
                assert_eq!(
                    finish.get_str("started_at").unwrap(),
                    received_at,
                    "startup drain reset the signal time"
                );
                assert_eq!(finish.get_i64("deadline_secs").unwrap(), 3600);
            }
            if step != "serving" {
                assert!(!transcript.contains("bound=true"), "{transcript}");
            }
        }
    }
}

#[tokio::test]
async fn startup_signal_at_entry() {
    check_step("entry", false, false).await;
}
#[tokio::test]
async fn startup_signal_after_database() {
    check_step("database_ready", false, false).await;
}
#[tokio::test]
async fn startup_signal_after_workers_finishes_step_then_drains() {
    check_step("after_workers", false, false).await;
}
#[tokio::test]
async fn startup_signal_before_listeners_drains_without_binding() {
    check_step("before_listeners", false, false).await;
}
#[tokio::test]
async fn startup_signal_second_during_startup_finishes_step_then_cleans_up() {
    check_step("after_workers", true, false).await;
}
#[tokio::test]
async fn startup_signal_after_listeners_drains() {
    check_step("serving", false, false).await;
}
#[tokio::test]
async fn startup_signal_one_shot_starts_no_workers() {
    check_step("one_shot", false, true).await;
}
#[tokio::test]
async fn startup_signal_one_shot_keeps_default_termination() {
    check_step("entry", false, true).await;
}
