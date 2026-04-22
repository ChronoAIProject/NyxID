use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use reqwest::header;
use rustls_pki_types::{CertificateDer, UnixTime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sigstore::cosign::{CosignCapabilities, client::Client as SigstoreClient};
use sigstore::trust::TrustRoot;
use sigstore::trust::sigstore::SigstoreTrustRoot;
use thiserror::Error;
use tokio::process::Command;
use x509_cert::Certificate;
use x509_cert::der::Decode;
use x509_cert::ext::pkix::SubjectAltName;
use x509_cert::ext::pkix::name::GeneralName;

const RELEASES_API_BASE: &str = "https://api.github.com";
const REPO_OWNER: &str = "ChronoAIProject";
const REPO_NAME: &str = "NyxID";
const MANIFEST_PATH: &str = "/cli/latest";
const SHA256SUMS_FILE: &str = "SHA256SUMS";
const SHA256SUMS_SIG_FILE: &str = "SHA256SUMS.sig";
const SHA256SUMS_CERT_FILE: &str = "SHA256SUMS.pem";
const SIGSTORE_ISSUER_OID: &str = "1.3.6.1.4.1.57264.1.1";
const CODE_SIGNING_EKU_OID: &[u8] = &[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x03];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CliReleaseManifest {
    pub version: String,
    pub commit: String,
    pub release_tag: String,
    pub release_url: String,
    pub asset_base_url: String,
    pub checksums_url: String,
    pub checksums_signature_url: String,
    pub checksums_cert_url: String,
    pub asset_name_template: String,
    pub cosign_identity: String,
    pub cosign_issuer: String,
}

impl CliReleaseManifest {
    pub fn for_version(&self, version: &str) -> Self {
        let version = normalize_version(version).to_string();
        let release_tag = format!("v{version}");
        let asset_base_url =
            format!("https://github.com/{REPO_OWNER}/{REPO_NAME}/releases/download/{release_tag}/");

        Self {
            version: version.clone(),
            commit: self.commit.clone(),
            release_tag: release_tag.clone(),
            release_url: format!(
                "https://github.com/{REPO_OWNER}/{REPO_NAME}/releases/tag/{release_tag}"
            ),
            asset_base_url: asset_base_url.clone(),
            checksums_url: format!("{asset_base_url}{SHA256SUMS_FILE}"),
            checksums_signature_url: format!("{asset_base_url}{SHA256SUMS_SIG_FILE}"),
            checksums_cert_url: format!("{asset_base_url}{SHA256SUMS_CERT_FILE}"),
            asset_name_template: self.asset_name_template.clone(),
            cosign_identity: format!(
                "https://github.com/{REPO_OWNER}/{REPO_NAME}/.github/workflows/release.yml@refs/tags/v{version}"
            ),
            cosign_issuer: self.cosign_issuer.clone(),
        }
    }

    pub fn archive_name_for_target(&self, target: &str) -> Result<String> {
        let ext = archive_ext_for_target(target).ok_or_else(|| {
            anyhow!("No release archive format is configured for target '{target}'")
        })?;

        Ok(self
            .asset_name_template
            .replace("{version}", &self.version)
            .replace("{target}", target)
            .replace("{ext}", ext))
    }
}

#[derive(Clone, Debug)]
pub struct GithubReleaseAssets {
    pub archive: self_update::update::ReleaseAsset,
    pub checksums: self_update::update::ReleaseAsset,
    pub checksums_signature: self_update::update::ReleaseAsset,
    pub checksums_cert: self_update::update::ReleaseAsset,
}

#[derive(Debug, Error)]
pub enum ReleaseLookupError {
    #[error("Release v{version} is missing archive '{archive_name}'")]
    MissingArchive {
        version: String,
        archive_name: String,
    },
    #[error("Release v{version} was not found on GitHub")]
    ReleaseNotFound { version: String },
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug)]
pub enum VerificationFailure {
    Unavailable(anyhow::Error),
    Rejected(anyhow::Error),
}

#[derive(Clone, Debug)]
pub struct VerificationContext<'a> {
    pub blob: &'a [u8],
    pub certificate_pem: &'a str,
    pub signature: &'a str,
    pub expected_identity: &'a str,
    pub expected_issuer: &'a str,
}

pub async fn fetch_release_manifest(base_url: &str) -> Result<CliReleaseManifest> {
    let url = format!("{}{}", base_url.trim_end_matches('/'), MANIFEST_PATH);
    let client = crate::api::build_cli_http_client()?;

    client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("Failed to fetch CLI release manifest from {url}"))?
        .error_for_status()
        .with_context(|| format!("CLI release manifest request failed for {url}"))?
        .json::<CliReleaseManifest>()
        .await
        .with_context(|| format!("Failed to decode CLI release manifest from {url}"))
}

pub fn normalize_version(version: &str) -> &str {
    version.trim().trim_start_matches('v')
}

pub fn mapped_release_target(host_target: &str) -> Option<&'static str> {
    match host_target {
        "x86_64-unknown-linux-gnu" => Some("x86_64-unknown-linux-gnu"),
        "aarch64-unknown-linux-gnu" => Some("aarch64-unknown-linux-gnu"),
        "x86_64-apple-darwin" => Some("x86_64-apple-darwin"),
        "aarch64-apple-darwin" => Some("aarch64-apple-darwin"),
        "x86_64-pc-windows-msvc" => Some("x86_64-pc-windows-msvc"),
        "aarch64-pc-windows-msvc" => Some("aarch64-pc-windows-msvc"),
        _ => None,
    }
}

pub fn archive_ext_for_target(target: &str) -> Option<&'static str> {
    if target.ends_with("windows-msvc") {
        Some("zip")
    } else if target.ends_with("linux-gnu") || target.ends_with("apple-darwin") {
        Some("tar.xz")
    } else {
        None
    }
}

pub fn host_release_target() -> Option<&'static str> {
    mapped_release_target(self_update::get_target())
}

pub fn is_newer_version(current: &str, latest: &str) -> Result<bool> {
    self_update::version::bump_is_greater(normalize_version(current), normalize_version(latest))
        .map_err(|error| anyhow!(error))
}

pub fn parse_sha256sums(contents: &str) -> Result<HashMap<String, String>> {
    let mut sums = HashMap::new();

    for (index, line) in contents.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut parts = trimmed.split_whitespace();
        let checksum = parts
            .next()
            .ok_or_else(|| anyhow!("Malformed checksum line {}", index + 1))?;
        let file_name = parts
            .next()
            .ok_or_else(|| anyhow!("Malformed checksum line {}", index + 1))?;

        if parts.next().is_some() {
            bail!("Malformed checksum line {}", index + 1);
        }

        sums.insert(
            file_name.trim_start_matches('*').to_string(),
            checksum.to_string(),
        );
    }

    if sums.is_empty() {
        bail!("No checksums were found in {SHA256SUMS_FILE}");
    }

    Ok(sums)
}

pub fn sha256_file_hex(path: &Path) -> Result<String> {
    let file = File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 16 * 1024];

    loop {
        let read = reader
            .read(&mut buffer)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

pub async fn fetch_github_release_assets(
    version: &str,
    target: &str,
    archive_name: &str,
) -> std::result::Result<GithubReleaseAssets, ReleaseLookupError> {
    let version = normalize_version(version).to_string();
    let target = target.to_string();
    let archive_name = archive_name.to_string();

    tokio::task::spawn_blocking(move || {
        let releases = self_update::backends::github::ReleaseList::configure()
            .repo_owner(REPO_OWNER)
            .repo_name(REPO_NAME)
            .with_url(RELEASES_API_BASE)
            .with_target(&target)
            .build()
            .context("Failed to configure GitHub release lookup")?
            .fetch()
            .context("Failed to fetch GitHub release metadata")?;

        let release = releases
            .into_iter()
            .find(|release| release.version == version)
            .ok_or_else(|| ReleaseLookupError::ReleaseNotFound {
                version: version.clone(),
            })?;

        let find_asset = |name: &str| -> std::result::Result<
            self_update::update::ReleaseAsset,
            ReleaseLookupError,
        > {
            release
                .assets
                .iter()
                .find(|asset| asset.name == name)
                .cloned()
                .ok_or_else(|| {
                    if name == archive_name {
                        ReleaseLookupError::MissingArchive {
                            version: version.clone(),
                            archive_name: name.to_string(),
                        }
                    } else {
                        ReleaseLookupError::Other(anyhow!(
                            "Release v{version} is missing asset '{name}'"
                        ))
                    }
                })
        };

        Ok::<_, ReleaseLookupError>(GithubReleaseAssets {
            archive: find_asset(&archive_name)?,
            checksums: find_asset(SHA256SUMS_FILE)?,
            checksums_signature: find_asset(SHA256SUMS_SIG_FILE)?,
            checksums_cert: find_asset(SHA256SUMS_CERT_FILE)?,
        })
    })
    .await
    .context("GitHub release lookup task failed")
    .map_err(ReleaseLookupError::Other)?
}

pub async fn download_asset(
    asset: &self_update::update::ReleaseAsset,
    destination: &Path,
    show_progress: bool,
) -> Result<()> {
    let asset_url = asset.download_url.clone();
    let destination = destination.to_path_buf();

    tokio::task::spawn_blocking(move || {
        let mut output = File::create(&destination)
            .with_context(|| format!("Failed to create {}", destination.display()))?;
        let mut download = self_update::Download::from_url(&asset_url);
        download
            .show_progress(show_progress)
            .set_header(header::ACCEPT, "application/octet-stream".parse()?);
        download
            .download_to(&mut output)
            .with_context(|| format!("Failed to download {}", destination.display()))
    })
    .await
    .context("Download task failed")?
}

pub async fn extract_archive(archive_path: &Path, destination: &Path) -> Result<()> {
    let archive_path = archive_path.to_path_buf();
    let destination = destination.to_path_buf();

    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&destination)
            .with_context(|| format!("Failed to create {}", destination.display()))?;

        let file_name = archive_path
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(|| anyhow!("Invalid archive file name: {}", archive_path.display()))?;

        if file_name.ends_with(".zip") {
            self_update::Extract::from_source(&archive_path)
                .archive(self_update::ArchiveKind::Zip)
                .extract_into(&destination)
                .with_context(|| format!("Failed to extract {}", archive_path.display()))?;
        } else if file_name.ends_with(".tar.xz") {
            let archive_file = File::open(&archive_path)
                .with_context(|| format!("Failed to open {}", archive_path.display()))?;
            let decompressed = xz2::read::XzDecoder::new(archive_file);
            let mut archive = tar::Archive::new(decompressed);
            archive
                .unpack(&destination)
                .with_context(|| format!("Failed to extract {}", archive_path.display()))?;
        } else {
            bail!("Unsupported archive format for {}", archive_path.display());
        }

        Ok(())
    })
    .await
    .context("Archive extraction task failed")?
}

pub fn find_binary(root: &Path, binary_name: &str) -> Result<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    let expected = if cfg!(windows) && !binary_name.ends_with(".exe") {
        format!("{binary_name}.exe")
    } else {
        binary_name.to_string()
    };

    while let Some(path) = stack.pop() {
        let entries = std::fs::read_dir(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        for entry in entries {
            let entry = entry.with_context(|| format!("Failed to inspect {}", path.display()))?;
            let entry_path = entry.path();
            if entry_path.is_dir() {
                stack.push(entry_path);
                continue;
            }

            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name == expected)
            {
                return Ok(entry_path);
            }
        }
    }

    bail!(
        "Downloaded archive did not contain the expected binary '{}'",
        expected
    )
}

pub async fn verify_checksums_signature(
    context: &VerificationContext<'_>,
) -> std::result::Result<(), VerificationFailure> {
    match verify_with_cosign_cli(context).await {
        Ok(()) => return Ok(()),
        Err(VerificationFailure::Rejected(error)) => {
            return Err(VerificationFailure::Rejected(error));
        }
        Err(VerificationFailure::Unavailable(_)) => {}
    }

    verify_with_sigstore(context).await
}

pub fn handle_signature_verification_failure(
    failure: VerificationFailure,
    allow_unverified: bool,
) -> Result<bool> {
    match failure {
        VerificationFailure::Unavailable(error) => {
            if allow_unverified {
                eprintln!("Warning: checksum signature verification was skipped: {error:#}");
                Ok(false)
            } else {
                bail!(
                    "Checksum signature verification is unavailable: {error:#}\n\
                     Install `cosign` or rerun with --allow-unverified to bypass this check."
                );
            }
        }
        VerificationFailure::Rejected(error) => {
            bail!("Checksum signature verification failed: {error:#}");
        }
    }
}

async fn verify_with_cosign_cli(
    context: &VerificationContext<'_>,
) -> std::result::Result<(), VerificationFailure> {
    let temp_dir =
        tempfile::tempdir().map_err(|error| VerificationFailure::Unavailable(anyhow!(error)))?;
    let certificate_path = temp_dir.path().join("SHA256SUMS.pem");
    let signature_path = temp_dir.path().join("SHA256SUMS.sig");
    let blob_path = temp_dir.path().join("SHA256SUMS");

    std::fs::write(&certificate_path, context.certificate_pem)
        .map_err(|error| VerificationFailure::Rejected(anyhow!(error)))?;
    std::fs::write(&signature_path, context.signature)
        .map_err(|error| VerificationFailure::Rejected(anyhow!(error)))?;
    std::fs::write(&blob_path, context.blob)
        .map_err(|error| VerificationFailure::Rejected(anyhow!(error)))?;

    let output = Command::new("cosign")
        .args([
            "verify-blob",
            "--certificate-identity",
            context.expected_identity,
            "--certificate-oidc-issuer",
            context.expected_issuer,
            "--certificate",
            certificate_path.to_string_lossy().as_ref(),
            "--signature",
            signature_path.to_string_lossy().as_ref(),
            blob_path.to_string_lossy().as_ref(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn();

    let child = match output {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(VerificationFailure::Unavailable(anyhow!(
                "`cosign` is not installed"
            )));
        }
        Err(error) => {
            return Err(VerificationFailure::Unavailable(anyhow!(
                "Failed to start `cosign`: {error}"
            )));
        }
    };

    let output = child
        .wait_with_output()
        .await
        .map_err(|error| VerificationFailure::Unavailable(anyhow!(error)))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(VerificationFailure::Rejected(anyhow!(
            if stderr.is_empty() {
                format!("`cosign verify-blob` exited with status {}", output.status)
            } else {
                stderr
            }
        )))
    }
}

async fn verify_with_sigstore(
    context: &VerificationContext<'_>,
) -> std::result::Result<(), VerificationFailure> {
    let normalized_cert = normalize_certificate_pem(context.certificate_pem)
        .map_err(VerificationFailure::Rejected)?;

    SigstoreClient::verify_blob(&normalized_cert, context.signature.trim(), context.blob)
        .map_err(|error| VerificationFailure::Rejected(anyhow!(error)))?;

    verify_fulcio_certificate(
        &normalized_cert,
        context.expected_identity,
        context.expected_issuer,
    )
    .await
}

async fn verify_fulcio_certificate(
    certificate_pem: &str,
    expected_identity: &str,
    expected_issuer: &str,
) -> std::result::Result<(), VerificationFailure> {
    let pem_certs =
        parse_certificate_chain(certificate_pem).map_err(VerificationFailure::Rejected)?;
    let (leaf, intermediates) = pem_certs
        .split_first()
        .ok_or_else(|| VerificationFailure::Rejected(anyhow!("No certificate was provided")))?;

    let trust_root = SigstoreTrustRoot::new(None)
        .await
        .map_err(|error| VerificationFailure::Unavailable(anyhow!(error)))?;
    let fulcio_certs = trust_root
        .fulcio_certs()
        .map_err(|error| VerificationFailure::Unavailable(anyhow!(error)))?;

    verify_certificate_chain(leaf, intermediates, &fulcio_certs)
        .map_err(VerificationFailure::Rejected)?;
    verify_certificate_identity(leaf, expected_identity, expected_issuer)
        .map_err(VerificationFailure::Rejected)?;

    Ok(())
}

fn normalize_certificate_pem(certificate_pem: &str) -> Result<String> {
    match base64::engine::general_purpose::STANDARD.decode(certificate_pem.trim()) {
        Ok(decoded) => {
            String::from_utf8(decoded).context("Decoded certificate was not valid UTF-8 PEM text")
        }
        Err(_) => Ok(certificate_pem.to_string()),
    }
}

fn parse_certificate_chain(certificate_pem: &str) -> Result<Vec<CertificateDer<'static>>> {
    let mut reader = certificate_pem.as_bytes();
    let certificates = rustls_pemfile::certs(&mut reader)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to parse PEM certificate bundle")?;

    if certificates.is_empty() {
        bail!("Certificate bundle was empty");
    }

    Ok(certificates)
}

fn verify_certificate_chain(
    leaf: &CertificateDer<'_>,
    intermediates: &[CertificateDer<'_>],
    roots: &[CertificateDer<'_>],
) -> Result<()> {
    let anchors = roots
        .iter()
        .map(|cert| webpki::anchor_from_trusted_cert(cert).map(|anchor| anchor.to_owned()))
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to construct Fulcio trust anchors")?;

    let end_entity = webpki::EndEntityCert::try_from(leaf)
        .context("Failed to parse leaf signing certificate")?;
    let verification_time = verification_time_for_leaf(leaf)?;

    end_entity
        .verify_for_usage(
            webpki::ALL_VERIFICATION_ALGS,
            &anchors,
            intermediates,
            verification_time,
            webpki::KeyUsage::required(CODE_SIGNING_EKU_OID),
            None,
            None,
        )
        .context("Signing certificate is not chained to Fulcio")?;

    Ok(())
}

fn verification_time_for_leaf(leaf: &CertificateDer<'_>) -> Result<UnixTime> {
    let certificate = Certificate::from_der(leaf.as_ref())
        .context("Failed to parse leaf certificate for validity window")?;
    Ok(UnixTime::since_unix_epoch(
        certificate
            .tbs_certificate
            .validity
            .not_before
            .to_unix_duration(),
    ))
}

fn verify_certificate_identity(
    leaf: &CertificateDer<'_>,
    expected_identity: &str,
    expected_issuer: &str,
) -> Result<()> {
    let certificate =
        Certificate::from_der(leaf.as_ref()).context("Failed to parse signing certificate")?;
    let subject = certificate_subject_uri(&certificate)?;
    if subject != expected_identity {
        bail!(
            "Signing certificate identity mismatch: expected '{expected_identity}', got '{subject}'"
        );
    }

    let issuer = certificate_extension_value(&certificate, SIGSTORE_ISSUER_OID)?
        .ok_or_else(|| anyhow!("Signing certificate is missing the Sigstore issuer extension"))?;
    if issuer != expected_issuer {
        bail!("Signing certificate issuer mismatch: expected '{expected_issuer}', got '{issuer}'");
    }

    Ok(())
}

fn certificate_subject_uri(certificate: &Certificate) -> Result<String> {
    let (_, san) = certificate
        .tbs_certificate
        .get::<SubjectAltName>()
        .context("Failed to read signing certificate subjectAltName")?
        .ok_or_else(|| anyhow!("Signing certificate is missing subjectAltName"))?;

    for name in &san.0 {
        if let GeneralName::UniformResourceIdentifier(uri) = name {
            return Ok(uri.to_string());
        }
    }

    bail!("Signing certificate did not contain a URI identity")
}

fn certificate_extension_value(certificate: &Certificate, oid: &str) -> Result<Option<String>> {
    let Some(extensions) = certificate.tbs_certificate.extensions.as_ref() else {
        return Ok(None);
    };

    let Some(extension) = extensions
        .iter()
        .find(|extension| extension.extn_id.to_string() == oid)
    else {
        return Ok(None);
    };

    Ok(Some(
        String::from_utf8(extension.extn_value.clone().into_bytes())
            .context("Signing certificate extension was not valid UTF-8")?,
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        VerificationFailure, archive_ext_for_target, handle_signature_verification_failure,
        is_newer_version, mapped_release_target, normalize_version, parse_sha256sums,
    };

    #[test]
    fn maps_supported_host_targets() {
        assert_eq!(
            mapped_release_target("x86_64-unknown-linux-gnu"),
            Some("x86_64-unknown-linux-gnu")
        );
        assert_eq!(
            mapped_release_target("aarch64-pc-windows-msvc"),
            Some("aarch64-pc-windows-msvc")
        );
        assert_eq!(mapped_release_target("x86_64-unknown-linux-musl"), None);
    }

    #[test]
    fn archive_extension_matches_platform() {
        assert_eq!(
            archive_ext_for_target("x86_64-unknown-linux-gnu"),
            Some("tar.xz")
        );
        assert_eq!(
            archive_ext_for_target("x86_64-apple-darwin"),
            Some("tar.xz")
        );
        assert_eq!(
            archive_ext_for_target("x86_64-pc-windows-msvc"),
            Some("zip")
        );
    }

    #[test]
    fn parses_sha256sums_file() {
        let parsed = parse_sha256sums(
            "abc123  nyxid-0.2.0-x86_64-unknown-linux-gnu.tar.xz\n\
             def456 *nyxid-0.2.0-x86_64-pc-windows-msvc.zip\n",
        )
        .expect("checksums should parse");

        assert_eq!(
            parsed
                .get("nyxid-0.2.0-x86_64-unknown-linux-gnu.tar.xz")
                .map(String::as_str),
            Some("abc123")
        );
        assert_eq!(
            parsed
                .get("nyxid-0.2.0-x86_64-pc-windows-msvc.zip")
                .map(String::as_str),
            Some("def456")
        );
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer_version("0.2.0", "0.3.0").expect("version should compare"));
        assert!(!is_newer_version("0.2.0", "0.2.0").expect("version should compare"));
        assert!(!is_newer_version("0.3.0", "0.2.0").expect("version should compare"));
    }

    #[test]
    fn strips_optional_version_prefix() {
        assert_eq!(normalize_version("v1.2.3"), "1.2.3");
        assert_eq!(normalize_version("1.2.3"), "1.2.3");
    }

    #[test]
    fn rejects_signature_mismatch_even_with_override() {
        let result = handle_signature_verification_failure(
            VerificationFailure::Rejected(anyhow::anyhow!("signature mismatch")),
            true,
        );

        assert!(result.is_err());
    }
}
