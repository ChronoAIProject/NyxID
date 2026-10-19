use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::api::{ApiClient, ApiError};

pub(super) const CREDENTIAL_PREFIX: &str = "nyx_owi_";
const RENEWAL_REQUIRED: i64 = 11016;
const PENDING_FILE: &str = "enrollment-credential.pending";

#[derive(Deserialize, Serialize, PartialEq, Eq)]
struct EnrollmentContext {
    base_url: String,
    pool_id: String,
    installation_id: String,
}

#[derive(Serialize)]
struct EnrollmentRequest<'a> {
    installation_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<&'a str>,
    credential: &'a str,
}

#[derive(Deserialize)]
struct EnrollmentResponse {
    pool_id: String,
    pool_slug: String,
    label: String,
    installation_id: String,
    credential_type: String,
}

pub(super) struct Enrollment {
    pub label: String,
    pub credential: Zeroizing<String>,
}

pub(super) fn valid_credential(value: &str) -> bool {
    value.strip_prefix(CREDENTIAL_PREFIX).is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn new_credential() -> Zeroizing<String> {
    let mut bytes = Zeroizing::new([0_u8; 32]);
    rand::thread_rng().fill_bytes(bytes.as_mut());
    let encoded = Zeroizing::new(hex::encode(bytes.as_slice()));
    Zeroizing::new(format!("{CREDENTIAL_PREFIX}{}", encoded.as_str()))
}

pub(super) fn lock_install(directory: &Path) -> Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let lock = options.open(directory.join(".install.lock"))?;
    lock.try_lock().context(
        "Another Oracle worker install is running for this pool/profile; retry when it finishes",
    )?;
    Ok(lock)
}

pub(super) fn write_secret(path: &Path, value: &[u8]) -> Result<()> {
    let parent = path.parent().context("Invalid worker credential path")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    temp.write_all(value)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

pub(super) async fn enroll(
    api: &mut ApiClient,
    pool: &str,
    pool_id: &str,
    directory: &Path,
    installation_id: &str,
    label: Option<&str>,
    installed_credential: Option<&str>,
) -> Result<Enrollment> {
    let context_path = directory.join("enrollment.json");
    let expected_context = EnrollmentContext {
        base_url: api.base_url_root().trim_end_matches('/').to_owned(),
        pool_id: pool_id.to_owned(),
        installation_id: installation_id.to_owned(),
    };
    match fs::read(&context_path) {
        Ok(bytes) => {
            let context: EnrollmentContext =
                serde_json::from_slice(&bytes).context("Invalid local worker enrollment record")?;
            if context != expected_context {
                bail!(
                    "This worker enrollment belongs to a different server, pool, or installation; use a separate --profile"
                )
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            write_secret(&context_path, &serde_json::to_vec(&expected_context)?)?;
        }
        Err(error) => return Err(error).context("Could not read local worker enrollment record"),
    }
    let pending = directory.join(PENDING_FILE);
    let mut credential = match fs::read_to_string(&pending) {
        Ok(value) => Zeroizing::new(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let value = installed_credential
                .map(|value| Zeroizing::new(value.to_owned()))
                .unwrap_or_else(new_credential);
            write_secret(&pending, value.as_bytes())?;
            value
        }
        Err(error) => return Err(error).context("Could not read pending worker enrollment"),
    };
    if !valid_credential(&credential) {
        bail!("The stored worker enrollment credential is invalid; restore the installation files")
    }
    let path = format!("/oracle/pools/{}/workers/enroll", urlencoding::encode(pool));
    for attempt in 0..2 {
        let response: Result<EnrollmentResponse> = api
            .post(
                &path,
                &EnrollmentRequest {
                    installation_id,
                    label,
                    credential: &credential,
                },
            )
            .await;
        match response {
            Ok(response) => {
                if response.pool_id != pool_id
                    || response.pool_slug.is_empty()
                    || response.installation_id != installation_id
                    || response.credential_type != "installation"
                    || response.label.is_empty()
                    || response.label.len() > 64
                    || !response
                        .label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                    || label.is_some_and(|label| label != response.label)
                {
                    bail!("Server returned an unexpected worker enrollment identity")
                }
                return Ok(Enrollment {
                    label: response.label,
                    credential,
                });
            }
            Err(error)
                if attempt == 0
                    && error
                        .downcast_ref::<ApiError>()
                        .filter(|error| error.status() == reqwest::StatusCode::CONFLICT)
                        .and_then(|error| error.response())
                        .is_some_and(|body| body.error_code == RENEWAL_REQUIRED) =>
            {
                credential = new_credential();
                write_secret(&pending, credential.as_bytes())?;
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("enrollment returns after its final attempt")
}

pub(super) fn complete(directory: &Path) -> Result<()> {
    match fs::remove_file(directory.join(PENDING_FILE)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("Could not finish local worker enrollment"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    const INSTALLATION: &str = "346f9870-7ea4-41e9-b5d4-879bf08095c3";

    fn success() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "pool_id": "pool-1", "pool_slug": "org-pool", "label": "my-worker",
            "installation_id": INSTALLATION, "credential_type": "installation",
        }))
    }

    fn client(server: &MockServer) -> ApiClient {
        ApiClient::new(&server.uri(), "test-session".into())
            .unwrap()
            .for_credential_transfer()
            .unwrap()
    }

    #[tokio::test]
    async fn interrupted_enrollment_reuses_private_pending_credential() {
        let server = MockServer::start().await;
        let calls = AtomicUsize::new(0);
        Mock::given(method("POST"))
            .and(path("/api/v1/oracle/pools/org-pool/workers/enroll"))
            .respond_with(move |_: &Request| {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    ResponseTemplate::new(503)
                } else {
                    success()
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        let directory = tempfile::tempdir().unwrap();
        let mut api = client(&server);
        assert!(
            enroll(
                &mut api,
                "org-pool",
                "pool-1",
                directory.path(),
                INSTALLATION,
                Some("my-worker"),
                None
            )
            .await
            .is_err()
        );
        let pending = fs::read_to_string(directory.path().join(PENDING_FILE)).unwrap();
        assert!(valid_credential(&pending));
        let enrolled = enroll(
            &mut api,
            "org-pool",
            "pool-1",
            directory.path(),
            INSTALLATION,
            Some("my-worker"),
            None,
        )
        .await
        .unwrap();
        assert!(enrolled.credential.as_str() == pending);
        assert!(!directory.path().join("worker-token").exists());
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests[0].body, requests[1].body);
        let body: serde_json::Value = requests[0].body_json().unwrap();
        assert_eq!(body["installation_id"], INSTALLATION);
        assert_eq!(body["label"], "my-worker");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(directory.path().join(PENDING_FILE))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        complete(directory.path()).unwrap();
        assert!(!directory.path().join(PENDING_FILE).exists());
    }

    #[tokio::test]
    async fn expired_authority_renews_once_without_overwriting_running_credential() {
        let server = MockServer::start().await;
        let calls = AtomicUsize::new(0);
        Mock::given(method("POST"))
            .respond_with(move |_: &Request| {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    ResponseTemplate::new(409).set_body_json(serde_json::json!({
                        "error":"renewal_required", "error_code":11016,
                        "message":"A new installation credential is required",
                    }))
                } else {
                    success()
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        let directory = tempfile::tempdir().unwrap();
        let original = new_credential();
        write_secret(&directory.path().join("worker-token"), original.as_bytes()).unwrap();
        let enrolled = enroll(
            &mut client(&server),
            "org-pool",
            "pool-1",
            directory.path(),
            INSTALLATION,
            Some("my-worker"),
            Some(&original),
        )
        .await
        .unwrap();
        assert!(enrolled.credential.as_str() != original.as_str());
        assert!(
            fs::read_to_string(directory.path().join("worker-token")).unwrap() == original.as_str()
        );
        let requests = server.received_requests().await.unwrap();
        let first: serde_json::Value = requests[0].body_json().unwrap();
        let second: serde_json::Value = requests[1].body_json().unwrap();
        assert!(first["credential"] != second["credential"]);
        assert!(second["credential"] == enrolled.credential.as_str());
    }

    #[tokio::test]
    async fn label_conflict_does_not_rotate_or_retry() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({
                "error":"label_unavailable", "error_code":11014, "message":"Label unavailable",
            })))
            .expect(1)
            .mount(&server)
            .await;
        let directory = tempfile::tempdir().unwrap();
        let original = new_credential();
        assert!(
            enroll(
                &mut client(&server),
                "org-pool",
                "pool-1",
                directory.path(),
                INSTALLATION,
                Some("my-worker"),
                Some(&original)
            )
            .await
            .is_err()
        );
        assert!(
            fs::read_to_string(directory.path().join(PENDING_FILE)).unwrap() == original.as_str()
        );
    }

    #[tokio::test]
    async fn enrollment_never_follows_a_credential_redirect() {
        let destination = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(success())
            .expect(0)
            .mount(&destination)
            .await;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(307).insert_header("Location", destination.uri()))
            .expect(1)
            .mount(&server)
            .await;
        let directory = tempfile::tempdir().unwrap();
        assert!(
            enroll(
                &mut client(&server),
                "org-pool",
                "pool-1",
                directory.path(),
                INSTALLATION,
                None,
                None
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn enrollment_rejects_a_different_pool_identity() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(success())
            .expect(1)
            .mount(&server)
            .await;
        let directory = tempfile::tempdir().unwrap();
        let error = enroll(
            &mut client(&server),
            "org-pool",
            "different-pool",
            directory.path(),
            INSTALLATION,
            None,
            None,
        )
        .await
        .err()
        .unwrap();
        assert!(
            error
                .to_string()
                .contains("unexpected worker enrollment identity")
        );
    }

    #[test]
    fn install_lock_serializes_identity_creation() {
        let directory = tempfile::tempdir().unwrap();
        let lock = lock_install(directory.path()).unwrap();
        assert!(lock_install(directory.path()).is_err());
        drop(lock);
        assert!(lock_install(directory.path()).is_ok());
    }

    #[tokio::test]
    async fn interrupted_enrollment_cannot_forward_the_credential_to_another_server() {
        let original = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&original)
            .await;
        let replacement = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(success())
            .expect(0)
            .mount(&replacement)
            .await;
        let directory = tempfile::tempdir().unwrap();
        assert!(
            enroll(
                &mut client(&original),
                "org-pool",
                "pool-1",
                directory.path(),
                INSTALLATION,
                None,
                None
            )
            .await
            .is_err()
        );
        let error = enroll(
            &mut client(&replacement),
            "org-pool",
            "pool-1",
            directory.path(),
            INSTALLATION,
            None,
            None,
        )
        .await
        .err()
        .unwrap();
        assert!(
            error
                .to_string()
                .contains("different server, pool, or installation")
        );
    }
}
