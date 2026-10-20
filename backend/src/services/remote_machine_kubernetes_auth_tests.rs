use super::*;

#[test]
fn metadata_configuration_does_not_require_a_token_file() {
    const CHILD: &str = "NYXID_METADATA_CONFIG_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "services::remote_machine_manager::kubernetes_auth_tests::metadata_configuration_does_not_require_a_token_file", "--nocapture"]);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("NYXID_REMOTE_MACHINE_") {
                command.env_remove(key);
            }
        }
        let output = command
            .env(CHILD, "1")
            .env("NYXID_REMOTE_MACHINE_KUBE_AUTH_MODE", "google_metadata")
            .env("NYXID_REMOTE_MACHINE_KUBE_API", "https://127.0.0.1:1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let readiness = platform_readiness("https://nyxid.test");
    assert!(
        !readiness
            .blockers
            .contains(&"manager_credential_file_unavailable")
    );
    assert!(
        !readiness
            .blockers
            .contains(&"kubernetes_identity_configuration_invalid")
    );
    assert!(sandbox_api_from_environment().is_ok());
}

use crate::services::remote_machine_bootstrap::BootstrapTicketVerifier;
use crate::services::remote_machine_kubernetes_identity::KubernetesWorkloadVerifier;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

const FACTORY_TEST: &str = "services::remote_machine_manager::kubernetes_auth_tests::kubernetes_auth_factories_share_cache_tls_readiness_and_pod_identity";
fn binding() -> nyxid_machine::managed::Binding {
    nyxid_machine::managed::Binding {
        node_id: "00000000-0000-4000-8000-000000000001".into(),
        sandbox_uid: "00000000-0000-4000-8000-000000000002".into(),
        pod_uid: "00000000-0000-4000-8000-000000000003".into(),
        writer_fence: "00000000-0000-4000-8000-000000000004".into(),
        operation_id: "00000000-0000-4000-8000-000000000005".into(),
        delivery_id: "00000000-0000-4000-8000-000000000006".into(),
        runtime_generation: 2,
        credential_epoch: 3,
    }
}
fn review() -> Value {
    json!({"status":{"authenticated":true,"audiences":["nyxid-remote-machine-v1"],"user":{
        "username":"system:serviceaccount:nyxid-machine:nyxid-machine-workload",
        "extra":{"authentication.kubernetes.io/pod-name":["machine-pod"],"authentication.kubernetes.io/pod-uid":[binding().pod_uid]}
    }}})
}
fn pod() -> Value {
    let b = binding();
    json!({"metadata":{"name":"machine-pod","namespace":"nyxid-machine","uid":b.pod_uid,
        "labels":{"nyxid.dev/managed":"true","nyxid.dev/machine-id":b.node_id,"nyxid.dev/runtime-generation":"2","nyxid.dev/credential-epoch":"3","nyxid.dev/writer-fence":b.writer_fence},
        "ownerReferences":[{"kind":"Sandbox","apiVersion":"agents.x-k8s.io/v1beta1","uid":b.sandbox_uid,"controller":true}]},
        "spec":{"serviceAccountName":"nyxid-machine-workload","automountServiceAccountToken":false}})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kubernetes_auth_factories_share_cache_tls_readiness_and_pod_identity() {
    const CHILD: &str = "NYXID_METADATA_FACTORY_TEST_CHILD";
    if let Ok(scenario) = std::env::var(CHILD) {
        factory_child(&scenario).await;
        return;
    }
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate.cert.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(certificate.signing_key.serialize_der().into()),
        )
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let captured = Arc::new(std::sync::Mutex::new(Vec::<(String, String, Value)>::new()));
    let seen = captured.clone();
    let denied = Arc::new(AtomicBool::new(false));
    let denied_server = denied.clone();
    let server = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let acceptor = acceptor.clone();
            let seen = seen.clone();
            let denied = denied_server.clone();
            tokio::spawn(async move {
                let Ok(mut stream) = acceptor.accept(stream).await else {
                    return;
                };
                let mut headers = Vec::new();
                while !headers.ends_with(b"\r\n\r\n") {
                    headers.push(stream.read_u8().await.unwrap());
                    assert!(headers.len() < 8192);
                }
                let headers = String::from_utf8(headers).unwrap();
                let path = headers
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .to_owned();
                let authorization = headers
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(k, _)| k.eq_ignore_ascii_case("authorization"))
                            .map(|(_, v)| v.trim().to_owned())
                    })
                    .unwrap();
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                            .map(|(_, v)| v.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                let mut bytes = vec![0; length];
                stream.read_exact(&mut bytes).await.unwrap();
                let body: Value = if bytes.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_slice(&bytes).unwrap()
                };
                let response = if path.ends_with("/tokenreviews") {
                    review()
                } else if path.ends_with("/pods/machine-pod") {
                    pod()
                } else if path.ends_with("/storageclasses/standard") {
                    json!({"kind":"StorageClass","apiVersion":"storage.k8s.io/v1","metadata":{"name":"standard","uid":"storage-uid"},"provisioner":"pd.csi.storage.gke.io"})
                } else {
                    json!({"major":"1"})
                };
                seen.lock().unwrap().push((path, authorization, body));
                let body = serde_json::to_string(&response).unwrap();
                let status = if denied.load(Ordering::SeqCst) {
                    "403 Forbidden"
                } else {
                    "200 OK"
                };
                stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            });
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let ca = dir.path().join("ca.pem");
    std::fs::write(&ca, certificate.cert.pem()).unwrap();
    let invalid_ca = dir.path().join("invalid.pem");
    std::fs::write(&invalid_ca, "invalid").unwrap();
    let token = dir.path().join("token");
    let metadata = MockServer::start().await;
    for scenario in [
        "metadata",
        "file",
        "default",
        "blank",
        "padded",
        "unknown",
        "missing-file",
        "relative-file",
        "http",
        "bad-ca",
        "untrusted-ca",
        "wrong-host",
        "unavailable",
        "denied",
    ] {
        metadata.reset().await;
        captured.lock().unwrap().clear();
        denied.store(scenario == "denied", Ordering::SeqCst);
        std::fs::write(&token, "file-fixture-one").unwrap();
        Mock::given(method("GET"))
            .respond_with(if scenario == "unavailable" {
                ResponseTemplate::new(503)
            } else {
                ResponseTemplate::new(200).set_body_json(
                    json!({"access_token":"google-fixture-secret","expires_in":3599}),
                )
            })
            .mount(&metadata)
            .await;
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", FACTORY_TEST, "--nocapture"]);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("NYXID_REMOTE_MACHINE_") {
                command.env_remove(key);
            }
        }
        command
            .env(CHILD, scenario)
            .env("NYXID_METADATA_FIXTURE_URL", metadata.uri())
            .env(
                "NYXID_REMOTE_MACHINE_KUBE_API",
                format!("https://localhost:{port}"),
            )
            .env("NYXID_REMOTE_MACHINE_KUBE_CA_FILE", &ca)
            .env("NYXID_REMOTE_MACHINE_KUBE_AUTH_MODE", "google_metadata")
            .env("NYXID_REMOTE_MACHINE_STORAGE_CLASS", "standard")
            .env(
                "NYXID_REMOTE_MACHINE_IMAGE_DIGEST",
                format!("registry.test/machine@sha256:{}", "a".repeat(64)),
            )
            // Deliberately unusable proxy: metadata must bypass all environment proxies.
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("http_proxy", "http://127.0.0.1:1")
            .env("ALL_PROXY", "http://127.0.0.1:1")
            .env("all_proxy", "http://127.0.0.1:1")
            .env("NO_PROXY", "localhost")
            .env("no_proxy", "localhost");
        match scenario {
            "file" | "default" => {
                command
                    .env("NYXID_REMOTE_MACHINE_KUBE_AUTH_MODE", "token_file")
                    .env("NYXID_REMOTE_MACHINE_KUBE_TOKEN_FILE", &token);
                if scenario == "default" {
                    command.env_remove("NYXID_REMOTE_MACHINE_KUBE_AUTH_MODE");
                }
            }
            "blank" => {
                command.env("NYXID_REMOTE_MACHINE_KUBE_AUTH_MODE", "");
            }
            "padded" => {
                command.env("NYXID_REMOTE_MACHINE_KUBE_AUTH_MODE", " google_metadata ");
            }
            "unknown" => {
                command.env("NYXID_REMOTE_MACHINE_KUBE_AUTH_MODE", "auto");
            }
            "missing-file" => {
                command.env("NYXID_REMOTE_MACHINE_KUBE_AUTH_MODE", "token_file");
            }
            "relative-file" => {
                command
                    .env("NYXID_REMOTE_MACHINE_KUBE_AUTH_MODE", "token_file")
                    .env("NYXID_REMOTE_MACHINE_KUBE_TOKEN_FILE", "relative");
            }
            "http" => {
                command.env(
                    "NYXID_REMOTE_MACHINE_KUBE_API",
                    format!("http://localhost:{port}"),
                );
            }
            "bad-ca" => {
                command.env("NYXID_REMOTE_MACHINE_KUBE_CA_FILE", &invalid_ca);
            }
            "untrusted-ca" => {
                command.env_remove("NYXID_REMOTE_MACHINE_KUBE_CA_FILE");
            }
            "wrong-host" => {
                command.env(
                    "NYXID_REMOTE_MACHINE_KUBE_API",
                    format!("https://127.0.0.1:{port}"),
                );
                command
                    .env("NO_PROXY", "localhost,127.0.0.1")
                    .env("no_proxy", "localhost,127.0.0.1");
            }
            _ => {}
        }
        if scenario == "unavailable" {
            command.env("NYXID_REMOTE_MACHINE_KUBE_TOKEN_FILE", &token);
        }
        let output = tokio::task::spawn_blocking(move || command.output().unwrap())
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{scenario}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let requests = metadata.received_requests().await.unwrap();
        let valid_metadata = matches!(
            scenario,
            "metadata" | "untrusted-ca" | "wrong-host" | "unavailable" | "denied"
        );
        assert_eq!(
            requests.len(),
            usize::from(valid_metadata),
            "metadata count: {scenario}"
        );
        if valid_metadata {
            assert_eq!(requests[0].headers["metadata-flavor"], "Google");
            assert_eq!(
                requests[0].url.path(),
                "/computeMetadata/v1/instance/service-accounts/default/token"
            );
        }
        let seen = captured.lock().unwrap();
        if scenario == "metadata" {
            assert!(seen.len() >= 7);
            assert!(
                seen.iter()
                    .all(|(_, auth, _)| auth == "Bearer google-fixture-secret")
            );
            let token_review = seen
                .iter()
                .find(|(path, _, _)| path.ends_with("/tokenreviews"))
                .unwrap();
            assert!(
                token_review.2["spec"]["token"]
                    .as_str()
                    .unwrap()
                    .starts_with("pod.")
            );
            assert_eq!(
                token_review.2["spec"]["audiences"],
                json!(["nyxid-remote-machine-v1"])
            );
        } else if matches!(scenario, "file" | "default") {
            assert!(seen.iter().any(|(_, a, _)| a == "Bearer file-fixture-one"));
            assert!(seen.iter().any(|(_, a, _)| a == "Bearer file-fixture-two"));
        } else if scenario != "denied" {
            assert!(seen.is_empty(), "unexpected Kubernetes request: {scenario}");
        }
    }
    server.abort();
}

async fn factory_child(scenario: &str) {
    // Only this test injects a loopback metadata fixture into the production
    // singleton. No endpoint override is compiled into environment loading.
    let endpoint = format!(
        "{}/computeMetadata/v1/instance/service-accounts/default/token",
        std::env::var("NYXID_METADATA_FIXTURE_URL").unwrap()
    );
    super::super::remote_machine_kubernetes_auth::GOOGLE_METADATA.set(Ok(
        nyxid_machine_updater::kubernetes_credentials::KubernetesCredentials::metadata_for_test(endpoint.parse().unwrap(), Duration::from_secs(3), Duration::from_secs(5), Duration::from_secs(1)).unwrap()
    )).unwrap();
    MANAGER_STARTED.store(true, Ordering::Release);
    let invalid = matches!(
        scenario,
        "blank" | "padded" | "unknown" | "missing-file" | "relative-file" | "http" | "bad-ca"
    );
    if invalid {
        assert!(sandbox_api_from_environment().is_err());
        assert!(KubernetesWorkloadVerifier::from_environment().is_err());
        assert!(
            platform_readiness("https://nyxid.test")
                .blockers
                .contains(&"kubernetes_identity_configuration_invalid")
        );
        return;
    }
    assert!(platform_readiness("https://nyxid.test").configuration_ready);
    let failed = matches!(
        scenario,
        "untrusted-ca" | "wrong-host" | "unavailable" | "denied"
    );
    for iteration in 0..2 {
        let api = sandbox_api_from_environment().unwrap();
        let verifier = KubernetesWorkloadVerifier::from_environment().unwrap();
        if iteration == 1 && matches!(scenario, "file" | "default") {
            std::fs::write(
                std::env::var("NYXID_REMOTE_MACHINE_KUBE_TOKEN_FILE").unwrap(),
                "file-fixture-two",
            )
            .unwrap();
        }
        if failed {
            assert!(api.resolve_storage_class(Some("standard")).await.is_err());
            assert!(verifier.read_json("/version").await.is_err());
        } else {
            assert_eq!(
                api.resolve_storage_class(Some("standard")).await.unwrap(),
                "standard"
            );
            assert_eq!(verifier.read_json("/version").await.unwrap()["major"], "1");
        }
    }
    let readiness = platform_readiness_for_provisioning("https://nyxid.test").await;
    assert_eq!(readiness.configuration_ready, !failed);
    assert!(
        !readiness
            .blockers
            .contains(&"manager_credential_file_unavailable")
    );
    if failed {
        assert!(
            readiness
                .blockers
                .contains(&"storage_class_unavailable_or_unverified")
        );
        return;
    }
    let token = format!(
        "pod.{}.fixture",
        URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(
                &json!({"exp":(chrono::Utc::now()+chrono::Duration::minutes(5)).timestamp()})
            )
            .unwrap()
        )
    );
    let ticket = json!({"token":token,"recipient_key_digest":"ab".repeat(32)});
    let verifier = KubernetesWorkloadVerifier::from_environment().unwrap();
    verifier.verify(&ticket, &binding()).await.unwrap();
}
