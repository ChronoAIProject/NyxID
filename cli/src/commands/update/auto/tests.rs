use super::*;
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicUsize, Ordering},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[test]
fn concurrent_manual_auto_and_rollback_share_one_lock() {
    let root = tempfile::tempdir().unwrap();
    let start = Arc::new(Barrier::new(8));
    let finish = Arc::new(Barrier::new(8));
    let winners = Arc::new(AtomicUsize::new(0));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let start = start.clone();
            let finish = finish.clone();
            let winners = winners.clone();
            let root = root.path();
            scope.spawn(move || {
                start.wait();
                let lock = lock_in(root).ok();
                if lock.is_some() {
                    winners.fetch_add(1, Ordering::SeqCst);
                }
                finish.wait();
                drop(lock);
            });
        }
    });
    assert_eq!(winners.load(Ordering::SeqCst), 1);
    assert!(lock_in(root.path()).is_ok());
}

#[tokio::test]
async fn disabled_held_and_not_due_runs_never_require_a_binary_or_network() {
    let root = tempfile::tempdir().unwrap();
    let mut policy = Policy::default();
    run_due(root.path(), &mut policy).await.unwrap();
    policy.enabled = true;
    policy.held_version = Some("v0.1.0".into());
    run_due(root.path(), &mut policy).await.unwrap();
    policy.held_version = None;
    policy.next_eligible_check = Some(Utc::now() + chrono::Duration::hours(24));
    run_due(root.path(), &mut policy).await.unwrap();
    assert!(policy.last_attempt.is_none());
    assert!(!root.path().join(".auto-update.json").exists());
    policy.next_eligible_check = Some(Utc::now() - chrono::Duration::seconds(1));
    assert!(run_due(root.path(), &mut policy).await.is_err());
    let persisted = read_in(root.path()).unwrap();
    assert!(persisted.last_attempt.is_some());
    assert_eq!(persisted.last_result.as_deref(), Some("failed"));
    assert!(persisted.next_eligible_check.unwrap() > Utc::now());
}

#[test]
fn scheduler_definitions_preserve_literal_paths_and_do_not_use_a_shell() {
    let root = Path::new("/tmp/space $dollar %percent \\\"quote/versions");
    let bin = Path::new("/tmp/space $dollar %percent \\\"quote/nyxid");
    let definition = launchd_plist(root, bin);
    let plist = plist::Value::from_reader_xml(definition.as_bytes()).unwrap();
    let values = plist.as_dictionary().unwrap();
    assert_eq!(
        values["ProgramArguments"].as_array().unwrap()[0].as_string(),
        root.join(".update-controller").to_str()
    );
    assert_eq!(
        values["EnvironmentVariables"].as_dictionary().unwrap()["NYXID_INSTALL_ROOT"].as_string(),
        root.to_str()
    );
    let unit = systemd_service(root, bin);
    let env = unit
        .lines()
        .find_map(|line| line.strip_prefix("Environment="))
        .unwrap();
    let tokens = shlex::split(env).unwrap();
    assert_eq!(
        tokens[0].replace("%%", "%"),
        format!("NYXID_INSTALL_ROOT={}", root.display())
    );
    let exec = unit
        .lines()
        .find_map(|line| line.strip_prefix("ExecStart="))
        .unwrap();
    let tokens = shlex::split(exec).unwrap();
    assert_eq!(
        tokens[0].replace("%%", "%").replace("$$", "$"),
        root.join(".update-controller").to_string_lossy()
    );
    assert_eq!(&tokens[1..], &["update", "auto", "run"]);
}

struct FixtureSource {
    client: reqwest::Client,
    url: String,
    valid: bool,
    verifications: AtomicUsize,
}
impl ReleaseSource for FixtureSource {
    async fn latest(&self) -> Result<Option<super::super::GitHubRelease>> {
        Ok(Some(
            self.client
                .get(&self.url)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?,
        ))
    }
    async fn verify(&self, archive: &Path, _: &str) -> Result<()> {
        assert!(archive.is_file());
        self.verifications.fetch_add(1, Ordering::SeqCst);
        anyhow::ensure!(self.valid, "invalid fixture attestation");
        Ok(())
    }
}

#[cfg(unix)]
fn installed_fixture(directory: &Path) -> (PathBuf, PathBuf, Policy) {
    let root = directory.join("versions");
    let old = root.join("v0.1.0/nyxid");
    fs::create_dir_all(old.parent().unwrap()).unwrap();
    fs::write(&old, b"old working binary").unwrap();
    let active = directory.join("bin/nyxid");
    super::super::retarget_active_symlink(&active, &old).unwrap();
    let policy = Policy {
        enabled: true,
        active_binary: Some(active.clone()),
        ..Policy::default()
    };
    (root, active, policy)
}

#[cfg(unix)]
#[tokio::test]
async fn already_current_offline_rate_limit_and_invalid_attestation_preserve_active_binary() {
    for mode in ["current", "offline", "rate_limited", "invalid_attestation"] {
        let tmp = tempfile::tempdir().unwrap();
        let (root, active, mut policy) = installed_fixture(tmp.path());
        let server = MockServer::start().await;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(500))
            .build()
            .unwrap();
        let url = if mode == "offline" {
            "http://127.0.0.1:1/release".into()
        } else {
            format!("{}/release", server.uri())
        };
        let source = FixtureSource {
            client: client.clone(),
            url,
            valid: false,
            verifications: AtomicUsize::new(0),
        };
        Mock::given(method("GET")).and(path("/release")).respond_with(if mode == "rate_limited" {
            ResponseTemplate::new(429)
        } else { ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "tag_name": if mode == "current" {"v0.1.0"} else {"v0.2.0"},
            "assets": [{"name": super::super::asset_name_for_target(super::super::current_target()).unwrap(), "browser_download_url": format!("{}/asset", server.uri())}]
        })) }).mount(&server).await;
        Mock::given(path("/asset"))
            .respond_with(
                ResponseTemplate::new(200).set_body_bytes(b"archive with invalid attestation"),
            )
            .mount(&server)
            .await;
        let result = perform_update_using(&root, &mut policy, &client, &source).await;
        if mode == "current" {
            assert_eq!(result.unwrap(), "already_current");
        } else {
            assert!(result.is_err());
        }
        assert_eq!(fs::read(&active).unwrap(), b"old working binary");
        assert!(!root.join("v0.2.0").exists());
        assert_eq!(
            source.verifications.load(Ordering::SeqCst),
            usize::from(mode == "invalid_attestation")
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn interrupted_download_never_reaches_verification_or_activation() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let tmp = tempfile::tempdir().unwrap();
    let (root, active, mut policy) = installed_fixture(tmp.path());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let asset = format!("http://{}/asset", listener.local_addr().unwrap());
    let transfer = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 1024];
        let _ = stream.read(&mut request).await.unwrap();
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 10000\r\nConnection: close\r\n\r\ninterrupted",
            )
            .await
            .unwrap();
    });
    let server = MockServer::start().await;
    Mock::given(path("/release")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "tag_name":"v0.2.0", "assets":[{"name":super::super::asset_name_for_target(super::super::current_target()).unwrap(),"browser_download_url":asset}]
    }))).mount(&server).await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let source = FixtureSource {
        client: client.clone(),
        url: format!("{}/release", server.uri()),
        valid: true,
        verifications: AtomicUsize::new(0),
    };
    assert!(
        perform_update_using(&root, &mut policy, &client, &source)
            .await
            .is_err()
    );
    transfer.await.unwrap();
    assert_eq!(source.verifications.load(Ordering::SeqCst), 0);
    assert_eq!(fs::read(active).unwrap(), b"old working binary");
}

#[cfg(unix)]
#[tokio::test]
async fn activated_version_and_pending_skills_survive_failed_skills_process() {
    let tmp = tempfile::tempdir().unwrap();
    let (root, active, mut policy) = installed_fixture(tmp.path());
    let archive = tmp.path().join("release.tar.gz");
    super::super::tests::release_tarball(&archive, b"#!/bin/sh\nexit 9\n");
    let server = MockServer::start().await;
    Mock::given(path("/release")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"tag_name":"v0.2.0", "assets":[{
        "name":super::super::asset_name_for_target(super::super::current_target()).unwrap(), "browser_download_url":format!("{}/asset",server.uri())}]}))).mount(&server).await;
    Mock::given(path("/asset"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(fs::read(archive).unwrap()))
        .mount(&server)
        .await;
    let client = reqwest::Client::new();
    let source = FixtureSource {
        client: client.clone(),
        url: format!("{}/release", server.uri()),
        valid: true,
        verifications: AtomicUsize::new(0),
    };
    assert!(
        perform_update_using(&root, &mut policy, &client, &source)
            .await
            .is_err()
    );
    let persisted = read_in(&root).unwrap();
    assert_eq!(persisted.last_activated_version.as_deref(), Some("v0.2.0"));
    assert_eq!(persisted.pending_skills_version.as_deref(), Some("v0.2.0"));
    assert!(persisted.retention_result.is_some());
    assert_eq!(fs::read_link(active).unwrap(), root.join("v0.2.0/nyxid"));
    assert!(root.join("v0.1.0/nyxid").is_file());
}

#[cfg(unix)]
#[tokio::test]
async fn verified_due_install_runs_exact_skills_command_and_retains_rollback() {
    let tmp = tempfile::tempdir().unwrap();
    let (root, active, mut policy) = installed_fixture(tmp.path());
    let archive = tmp.path().join("release.tar.gz");
    let binary = b"#!/bin/sh\n[ \"$#\" -eq 2 ] && [ \"$1\" = ai-setup ] && [ \"$2\" = update ] || exit 91\nprintf verified > \"$0.skills-result\"\n";
    super::super::tests::release_tarball(&archive, binary);
    let server = MockServer::start().await;
    Mock::given(path("/release")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "tag_name":"v0.2.0", "assets":[{"name":super::super::asset_name_for_target(super::super::current_target()).unwrap(),
        "browser_download_url":format!("{}/asset",server.uri())}]}))).expect(1).mount(&server).await;
    Mock::given(path("/asset"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(fs::read(archive).unwrap()))
        .expect(1)
        .mount(&server)
        .await;
    let client = reqwest::Client::new();
    let source = FixtureSource {
        client: client.clone(),
        url: format!("{}/release", server.uri()),
        valid: true,
        verifications: AtomicUsize::new(0),
    };
    assert_eq!(
        perform_update_using(&root, &mut policy, &client, &source)
            .await
            .unwrap(),
        "installed v0.2.0; skills refreshed; nodes deferred"
    );
    assert_eq!(source.verifications.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read(&active).unwrap(), binary);
    assert_eq!(
        fs::read(root.join("v0.2.0/nyxid.skills-result")).unwrap(),
        b"verified"
    );
    assert!(policy.pending_skills_version.is_none());
    assert!(root.join("v0.1.0/nyxid").is_file());
    assert_eq!(fs::read(root.join(".update-controller")).unwrap(), binary);
}

#[cfg(unix)]
#[tokio::test]
async fn failed_controller_refresh_retries_after_activation_without_redownload_or_repeating_skills()
{
    let tmp = tempfile::tempdir().unwrap();
    let (root, active, mut policy) = installed_fixture(tmp.path());
    fs::create_dir(root.join(".update-controller")).unwrap();
    let archive = tmp.path().join("release.tar.gz");
    let binary = b"#!/bin/sh\n[ \"$#\" -eq 2 ] && [ \"$1\" = ai-setup ] && [ \"$2\" = update ] || exit 91\n[ ! -f \"$0.skills-result\" ] || exit 92\nprintf verified > \"$0.skills-result\"\n";
    super::super::tests::release_tarball(&archive, binary);
    let server = MockServer::start().await;
    Mock::given(path("/release")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "tag_name":"v0.2.0", "assets":[{"name":super::super::asset_name_for_target(super::super::current_target()).unwrap(),
        "browser_download_url":format!("{}/asset",server.uri())}]}))).expect(1).mount(&server).await;
    Mock::given(path("/asset"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(fs::read(archive).unwrap()))
        .expect(1)
        .mount(&server)
        .await;
    let client = reqwest::Client::new();
    let source = FixtureSource {
        client: client.clone(),
        url: format!("{}/release", server.uri()),
        valid: true,
        verifications: AtomicUsize::new(0),
    };
    assert!(
        perform_update_using(&root, &mut policy, &client, &source)
            .await
            .unwrap_err()
            .to_string()
            .contains("activated")
    );
    assert_eq!(fs::read(&active).unwrap(), binary);
    assert_eq!(policy.pending_controller_version.as_deref(), Some("v0.2.0"));
    assert!(policy.pending_skills_version.is_none());
    assert_eq!(policy.retention_result.as_deref(), Some("complete"));
    fs::remove_dir(root.join(".update-controller")).unwrap();
    // A persisted failed cleanup phase must reconcile on the same release too.
    policy.retention_result = Some("failed; retry pending".into());
    write_in(&root, &policy).unwrap();
    let mut policy = read_in(&root).unwrap();
    assert!(
        perform_update_using(&root, &mut policy, &client, &source)
            .await
            .unwrap()
            .contains("pending phases completed")
    );
    assert!(!pending_phases(&policy));
    assert_eq!(fs::read(root.join(".update-controller")).unwrap(), binary);
}

#[cfg(unix)]
#[tokio::test]
async fn activation_intent_recovers_crash_before_completion_marker_without_network() {
    let tmp = tempfile::tempdir().unwrap();
    let (root, active, mut policy) = installed_fixture(tmp.path());
    let archive = tmp.path().join("release.tar.gz");
    let binary = b"#!/bin/sh\n[ \"$1\" = ai-setup ] && [ \"$2\" = update ] || exit 91\nprintf done > \"$0.skills-result\"\n";
    super::super::tests::release_tarball(&archive, binary);
    let versioned =
        super::super::extract_binary_to_version_root(&archive, "v0.2.0", &root).unwrap();
    policy.activation_intent = Some("v0.2.0".into());
    write_in(&root, &policy).unwrap();
    super::super::retarget_active_symlink(&active, &versioned).unwrap();
    // The process ended after activation, before persisting any completion phase.
    let mut policy = read_in(&root).unwrap();
    assert!(policy.last_activated_version.is_none());
    let client = reqwest::Client::new();
    let source = FixtureSource {
        client: client.clone(),
        url: "http://127.0.0.1:1/offline".into(),
        valid: false,
        verifications: AtomicUsize::new(0),
    };
    assert!(
        perform_update_using(&root, &mut policy, &client, &source)
            .await
            .unwrap()
            .contains("pending phases completed")
    );
    assert!(policy.activation_intent.is_none());
    assert_eq!(policy.last_activated_version.as_deref(), Some("v0.2.0"));
    assert!(!pending_phases(&policy));
    assert_eq!(
        fs::read(versioned.with_extension("skills-result")).unwrap(),
        b"done"
    );
    assert_eq!(fs::read(root.join(".update-controller")).unwrap(), binary);
}

#[cfg(unix)]
#[test]
fn activation_intent_before_failed_switch_does_not_claim_installation() {
    let tmp = tempfile::tempdir().unwrap();
    let (root, active, mut policy) = installed_fixture(tmp.path());
    policy.activation_intent = Some("v0.2.0".into());
    write_in(&root, &policy).unwrap();
    let mut policy = read_in(&root).unwrap();
    reconcile_activation(&root, &mut policy, &active_version(&root, &active).unwrap()).unwrap();
    assert!(policy.last_activated_version.is_none());
    assert!(!pending_phases(&policy));
    assert!(policy.last_result.unwrap().contains("did not complete"));
    assert_eq!(fs::read(active).unwrap(), b"old working binary");
}
