use std::time::Duration;

use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use const_oid::{
    ObjectIdentifier,
    db::{rfc5280::ID_KP_CODE_SIGNING, rfc6962::CT_PRECERT_SCTS},
};
use pki_types::{CertificateDer, UnixTime};
use reqwest::header::CONTENT_TYPE;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sigstore::crypto::{CosignVerificationKey, Signature};
use webpki::{EndEntityCert, KeyUsage};
use x509_cert::Certificate;
use x509_cert::der::{Decode, Encode};
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::ext::pkix::sct::{HashAlgorithm, SignatureAlgorithm, SignedCertificateTimestamp};
use x509_cert::ext::pkix::{SignedCertificateTimestampList, SubjectAltName};

#[path = "update_attestation/rekor.rs"]
mod rekor;
#[path = "update_attestation/trust.rs"]
mod trust;

const GITHUB_API_URL: &str = "https://api.github.com";
const MAX_ATTESTATIONS: usize = 30;
const MAX_API_BYTES: usize = 8 * 1024 * 1024;
const MAX_BUNDLE_BYTES: usize = 512 * 1024;
const MAX_PAYLOAD_BYTES: usize = 256 * 1024;
const MAX_CERTIFICATE_BYTES: usize = 16 * 1024;

// Shared by machine-updater's server verifier; unused in the native CLI build.
#[allow(dead_code)]
pub(crate) fn prepare_tuf_datastore(path: &std::path::Path) -> Result<()> {
    trust::prepare_datastore(path)
}

pub(crate) async fn read_json_bounded<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<T> {
    anyhow::ensure!(
        response.content_length().is_none_or(|n| n <= limit as u64),
        "JSON response exceeds size limit"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            chunk.len() <= limit.saturating_sub(bytes.len()),
            "JSON response exceeds size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&bytes)?)
}

pub(crate) async fn verify_release_attestation(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    expected_digest: &str,
    expected_identity: &str,
) -> Result<()> {
    let attestations = fetch_github_attestations(client, owner, repo, expected_digest, false)
        .await
        .with_context(|| {
            format!("Failed to fetch GitHub attestations for sha256:{expected_digest}")
        })?;

    if attestations.is_empty() {
        anyhow::bail!("No GitHub artifact attestations found for sha256:{expected_digest}");
    }

    let trust_root = trust::load()
        .await
        .context("Failed to load Sigstore public-good trust root")?;
    let mut failures = Vec::new();

    for attestation in &attestations {
        match verify_single_attestation(
            attestation,
            &trust_root,
            expected_digest,
            expected_identity,
        ) {
            Ok(()) => return Ok(()),
            Err(err) => failures.push(format!("{err:#}")),
        }
    }

    anyhow::bail!(
        "No valid release attestation matched expected workflow identity {expected_identity}. Verification failures: {}",
        failures.join(" | ")
    );
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ImageVerificationError {
    #[error("attestation_unavailable")]
    AttestationUnavailable,
    #[error("trust_root_unavailable")]
    TrustRootUnavailable,
    #[error("attestation_invalid")]
    AttestationInvalid,
}

/// Image updates use the same certificate, Rekor, DSSE and digest verifier as
/// CLI releases, with TUF metadata mirrored on GitHub (no third-party egress).
pub(crate) async fn verify_image_attestation(
    client: &reqwest::Client,
    digest: &str,
    version: &str,
    datastore: Option<&std::path::Path>,
) -> Result<(), ImageVerificationError> {
    let attestations = fetch_github_attestations(client, "ChronoAIProject", "NyxID", digest, true)
        .await
        .map_err(|_| ImageVerificationError::AttestationUnavailable)?;
    let root = trust::load_github(client.clone(), datastore)
        .await
        .map_err(|_| ImageVerificationError::TrustRootUnavailable)?;
    let identity = format!(
        "https://github.com/ChronoAIProject/NyxID/.github/workflows/publish-images.yml@refs/tags/v{version}"
    );
    for attestation in attestations {
        if verify_single_attestation(&attestation, &root, digest, &identity).is_ok() {
            return Ok(());
        }
    }
    Err(ImageVerificationError::AttestationInvalid)
}

async fn fetch_github_attestations(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    digest: &str,
    no_proxy: bool,
) -> Result<Vec<Attestation>> {
    let url = format!("{GITHUB_API_URL}/repos/{owner}/{repo}/attestations/sha256:{digest}");
    let mut request = client
        .get(&url)
        .query(&[("per_page", MAX_ATTESTATIONS.to_string())]);
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        let token = zeroize::Zeroizing::new(token);
        request = request.bearer_auth(token.as_str());
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("Failed to query GitHub attestation API: {url}"))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new());
    }

    let response = response
        .error_for_status()
        .with_context(|| format!("GitHub attestation API returned an error for {url}"))?;
    let body = read_json_bounded::<AttestationsResponse>(response, MAX_API_BYTES)
        .await
        .context("Failed to parse GitHub attestation response")?;
    anyhow::ensure!(
        body.attestations.len() <= MAX_ATTESTATIONS,
        "Too many attestations in response"
    );

    let mut attestations = Vec::new();
    for attestation in body.attestations {
        if attestation.bundle.is_some() {
            attestations.push(attestation);
        } else if let Some(bundle_url) = &attestation.bundle_url {
            let bundle = fetch_bundle_url(bundle_url, no_proxy).await?;
            attestations.push(Attestation {
                bundle: Some(bundle),
                bundle_url: attestation.bundle_url.clone(),
            });
        }
    }

    Ok(attestations)
}

fn allowed_bundle_url(parsed: &url::Url) -> bool {
    parsed.scheme() == "https"
        && parsed.port_or_known_default() == Some(443)
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && matches!(
            parsed.host_str(),
            Some(
                "api.github.com"
                    | "github.com"
                    | "objects.githubusercontent.com"
                    | "release-assets.githubusercontent.com"
            )
        )
}

fn bundle_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() > 3 || !allowed_bundle_url(attempt.url()) {
            attempt.error("Untrusted attestation redirect")
        } else {
            attempt.follow()
        }
    })
}

async fn fetch_bundle_url(bundle_url: &str, no_proxy: bool) -> Result<SigstoreBundle> {
    let parsed = url::Url::parse(bundle_url)?;
    anyhow::ensure!(
        allowed_bundle_url(&parsed),
        "Untrusted attestation bundle URL"
    );
    // Do not inherit a caller's default Authorization header. Check every
    // redirect before requesting it, rather than only checking the final URL.
    let builder = crate::tls::client_builder()?
        .user_agent("nyxid-attestation")
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(bundle_redirect_policy());
    // Machine companions retain their direct-only GitHub transport contract.
    let client = if no_proxy {
        builder.no_proxy()
    } else {
        builder
    }
    .build()?;
    let response = client
        .get(bundle_url)
        .send()
        .await
        .with_context(|| format!("Failed to fetch attestation bundle: {bundle_url}"))?;
    let response = response
        .error_for_status()
        .with_context(|| format!("GitHub returned an error for attestation bundle {bundle_url}"))?;

    let is_snappy = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        == Some("application/x-snappy");
    if is_snappy {
        anyhow::bail!(
            "GitHub returned a snappy-compressed attestation bundle, which this updater does not support"
        );
    }

    read_json_bounded::<SigstoreBundle>(response, MAX_BUNDLE_BYTES)
        .await
        .context("Failed to parse downloaded attestation bundle")
}

fn verify_single_attestation(
    attestation: &Attestation,
    trust_root: &trust::Root,
    expected_digest: &str,
    expected_identity: &str,
) -> Result<()> {
    let parsed = parse_bundle(attestation)?;
    let signing_time = rekor::verify(&parsed, trust_root)?;
    let certificate =
        verify_certificate_chain(&parsed.certificate_chain, trust_root, signing_time)?;

    verify_attestation_identity(&certificate, expected_identity)?;
    verify_payload_subject_digest(&parsed.payload, expected_digest)?;
    verify_dsse_signature(&certificate, &parsed.envelope, &parsed.payload)?;

    Ok(())
}

fn parse_bundle(attestation: &Attestation) -> Result<ParsedBundle> {
    let bundle = attestation
        .bundle
        .as_ref()
        .context("Attestation record did not include a Sigstore bundle")?;
    let require_proof = match bundle.media_type.as_deref() {
        Some("application/vnd.dev.sigstore.bundle+json;version=0.1") => false,
        Some("application/vnd.dev.sigstore.bundle+json;version=0.2")
        | Some("application/vnd.dev.sigstore.bundle+json;version=0.3")
        | Some("application/vnd.dev.sigstore.bundle.v0.3+json") => true,
        _ => anyhow::bail!("Unsupported Sigstore bundle format"),
    };
    let envelope = bundle
        .dsse_envelope
        .as_ref()
        .context("Attestation bundle did not include a DSSE envelope")?;
    anyhow::ensure!(
        envelope.payload.len() <= MAX_PAYLOAD_BYTES * 4 / 3 + 4
            && envelope.payload_type.len() <= 128
            && envelope.signatures.len() == 1
            && envelope.signatures[0].sig.len() <= 4096
            && envelope.signatures[0].keyid.len() <= 128,
        "Oversized or unsupported DSSE envelope"
    );
    let envelope = envelope.clone();
    let payload = BASE64
        .decode(&envelope.payload)
        .context("Failed to decode DSSE payload")?;
    let verification_material = bundle
        .verification_material
        .as_ref()
        .context("Attestation bundle did not include verification material")?;
    let certificate_chain = extract_certificate_chain(verification_material)?;
    let tlog_entries = extract_tlog_entries(verification_material)?;
    anyhow::ensure!(
        payload.len() <= MAX_PAYLOAD_BYTES,
        "DSSE payload exceeds size limit"
    );

    Ok(ParsedBundle {
        require_proof,
        envelope,
        payload,
        certificate_chain,
        tlog_entries,
    })
}

fn extract_certificate_chain(verification_material: &Value) -> Result<Vec<Vec<u8>>> {
    if let Some(raw_bytes) = verification_material
        .pointer("/certificate/rawBytes")
        .and_then(Value::as_str)
    {
        anyhow::ensure!(
            raw_bytes.len() <= MAX_CERTIFICATE_BYTES * 4 / 3 + 4,
            "Certificate exceeds size limit"
        );
        return Ok(vec![
            BASE64
                .decode(raw_bytes)
                .context("Failed to decode attestation certificate")?,
        ]);
    }

    if let Some(certificates) = verification_material
        .pointer("/x509CertificateChain/certificates")
        .and_then(Value::as_array)
    {
        anyhow::ensure!(
            !certificates.is_empty() && certificates.len() <= 8,
            "Unsupported certificate chain length"
        );
        let mut chain = Vec::new();
        for certificate in certificates {
            let raw_bytes = certificate
                .get("rawBytes")
                .and_then(Value::as_str)
                .context("Certificate chain entry missing rawBytes")?;
            anyhow::ensure!(
                raw_bytes.len() <= MAX_CERTIFICATE_BYTES * 4 / 3 + 4,
                "Certificate exceeds size limit"
            );
            chain.push(
                BASE64
                    .decode(raw_bytes)
                    .context("Failed to decode certificate chain entry")?,
            );
        }

        if !chain.is_empty() {
            return Ok(chain);
        }
    }

    anyhow::bail!("Attestation verification material did not include a signing certificate")
}

fn extract_tlog_entries(verification_material: &Value) -> Result<Vec<Value>> {
    if let Some(entries) = verification_material
        .get("tlogEntries")
        .and_then(Value::as_array)
    {
        anyhow::ensure!(entries.len() == 1, "Expected exactly one Rekor entry");
        return Ok(entries.clone());
    }

    if let Some(entry) = verification_material.get("rekorBundle") {
        return Ok(vec![entry.clone()]);
    }

    anyhow::bail!("Attestation verification material did not include a Rekor transparency entry")
}

fn verify_certificate_chain(
    certificate_chain: &[Vec<u8>],
    trust_root: &trust::Root,
    signing_time: UnixTime,
) -> Result<Certificate> {
    let leaf_der = certificate_chain
        .first()
        .context("Attestation certificate chain was empty")?;
    let leaf_certificate =
        Certificate::from_der(leaf_der).context("Failed to parse signing certificate")?;
    let leaf_der = CertificateDer::from(leaf_der.clone());
    let end_entity = EndEntityCert::try_from(&leaf_der)
        .context("Failed to parse signing certificate for chain verification")?;
    let fulcio_certs = trust_root
        .fulcio_certs(signing_time)
        .context("Failed to load Fulcio certificates from Sigstore trust root")?;
    let trust_anchors = fulcio_certs
        .iter()
        .map(|cert| webpki::anchor_from_trusted_cert(cert).map(|anchor| anchor.to_owned()))
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to build Fulcio trust anchors")?;
    let intermediates = certificate_chain
        .iter()
        .skip(1)
        .cloned()
        .map(CertificateDer::from)
        .collect::<Vec<_>>();

    let verified_path = end_entity
        .verify_for_usage(
            webpki::ALL_VERIFICATION_ALGS,
            &trust_anchors,
            &intermediates,
            signing_time,
            KeyUsage::required(ID_KP_CODE_SIGNING.as_bytes()),
            None,
            None,
        )
        .context("Signing certificate did not chain to the Sigstore Fulcio trust root")?;
    verify_embedded_sct(&leaf_certificate, &verified_path, trust_root, signing_time)?;

    Ok(leaf_certificate)
}

fn verify_embedded_sct(
    certificate: &Certificate,
    verified_path: &webpki::VerifiedPath<'_>,
    trust_root: &trust::Root,
    signing_time: UnixTime,
) -> Result<()> {
    let (_, sct_list) = certificate
        .tbs_certificate
        .get::<SignedCertificateTimestampList>()
        .context("Failed to parse signing certificate SCT extension")?
        .context("Signing certificate did not contain an embedded SCT")?;
    let timestamps = sct_list
        .parse_timestamps()
        .map_err(|err| anyhow::anyhow!("Failed to parse signing certificate SCT list: {err:?}"))?;
    let [serialized_sct] = timestamps.as_slice() else {
        anyhow::bail!(
            "Signing certificate SCT list must contain exactly one timestamp, found {}",
            timestamps.len()
        );
    };
    let sct = serialized_sct
        .parse_timestamp()
        .map_err(|err| anyhow::anyhow!("Failed to parse signing certificate SCT: {err:?}"))?;
    let sct_unix_secs = sct.timestamp / 1000;
    if sct_unix_secs > signing_time.as_secs() {
        anyhow::bail!("Signing certificate SCT timestamp is after the Rekor integrated timestamp");
    }

    let log_key_id = hex::encode(sct.log_id.key_id);
    let log_key = trust_root.ctfe_key(&log_key_id)?;
    log_key.check_millis(sct.timestamp)?;
    let issuer_spki = verified_path_issuer_spki(verified_path)?;
    let issuer_key_hash: [u8; 32] = Sha256::digest(&issuer_spki).into();
    let signed_payload = build_sct_signed_payload(certificate, &sct, &issuer_key_hash)?;

    let verification_key = CosignVerificationKey::try_from_der(&log_key.der)
        .context("Failed to parse Sigstore CT log public key")?;
    verify_sct_algorithm(&sct, &verification_key)?;
    verification_key
        .verify_signature(
            Signature::Raw(sct.signature.signature.as_slice()),
            &signed_payload,
        )
        .context("Signing certificate SCT signature did not verify against Sigstore CT log")?;

    Ok(())
}

fn verified_path_issuer_spki(verified_path: &webpki::VerifiedPath<'_>) -> Result<Vec<u8>> {
    if let Some(issuer) = verified_path.intermediate_certificates().next() {
        let issuer = Certificate::from_der(&issuer.der())
            .context("Failed to parse signing certificate issuer")?;
        return issuer
            .tbs_certificate
            .subject_public_key_info
            .to_der()
            .context("Failed to encode signing certificate issuer SPKI");
    }

    let body = &verified_path.anchor().subject_public_key_info;
    if body.len() > 127 {
        anyhow::bail!("Fulcio trust anchor SPKI is too large for SCT issuer encoding");
    }

    let mut spki = Vec::with_capacity(body.len() + 2);
    spki.push(0x30);
    spki.push(body.len() as u8);
    spki.extend_from_slice(body);
    Ok(spki)
}

fn build_sct_signed_payload(
    certificate: &Certificate,
    sct: &SignedCertificateTimestamp,
    issuer_key_hash: &[u8; 32],
) -> Result<Vec<u8>> {
    let mut tbs_precert = certificate.tbs_certificate.clone();
    tbs_precert.extensions = tbs_precert.extensions.map(|extensions| {
        extensions
            .iter()
            .filter(|extension| extension.extn_id != CT_PRECERT_SCTS)
            .cloned()
            .collect()
    });
    let tbs_precert_der = tbs_precert
        .to_der()
        .context("Failed to DER-encode SCT precertificate payload")?;

    let mut payload = Vec::new();
    payload.push(0);
    payload.push(0);
    payload.extend_from_slice(&sct.timestamp.to_be_bytes());
    payload.extend_from_slice(&1u16.to_be_bytes());
    payload.extend_from_slice(issuer_key_hash);
    write_u24(&mut payload, tbs_precert_der.len())?;
    payload.extend_from_slice(&tbs_precert_der);
    write_u16(&mut payload, sct.extensions.as_slice().len())?;
    payload.extend_from_slice(sct.extensions.as_slice());

    Ok(payload)
}

fn verify_sct_algorithm(
    sct: &SignedCertificateTimestamp,
    key: &CosignVerificationKey,
) -> Result<()> {
    match (
        &sct.signature.algorithm.hash,
        &sct.signature.algorithm.signature,
        key,
    ) {
        (
            HashAlgorithm::Sha256,
            SignatureAlgorithm::Ecdsa,
            CosignVerificationKey::ECDSA_P256_SHA256_ASN1(_),
        )
        | (
            HashAlgorithm::Sha384,
            SignatureAlgorithm::Ecdsa,
            CosignVerificationKey::ECDSA_P384_SHA384_ASN1(_),
        )
        | (
            HashAlgorithm::Sha256,
            SignatureAlgorithm::Rsa,
            CosignVerificationKey::RSA_PKCS1_SHA256(_),
        ) => Ok(()),
        _ => anyhow::bail!("Signing certificate SCT used an unsupported signature algorithm"),
    }
}

fn write_u16(out: &mut Vec<u8>, len: usize) -> Result<()> {
    let len = u16::try_from(len).context("SCT field exceeded u16 length")?;
    out.extend_from_slice(&len.to_be_bytes());
    Ok(())
}

fn write_u24(out: &mut Vec<u8>, len: usize) -> Result<()> {
    if len > 0x00ff_ffff {
        anyhow::bail!("SCT field exceeded u24 length");
    }

    out.push(((len >> 16) & 0xff) as u8);
    out.push(((len >> 8) & 0xff) as u8);
    out.push((len & 0xff) as u8);
    Ok(())
}

fn verify_attestation_identity(certificate: &Certificate, expected_identity: &str) -> Result<()> {
    // Fulcio authenticates identities for several OIDC providers. A matching
    // URI alone does not constrain which provider vouched for that identity.
    verify_github_issuer(certificate)?;
    let (_, san) = certificate
        .tbs_certificate
        .get::<SubjectAltName>()
        .context("Failed to parse certificate Subject Alternative Name")?
        .context("Attestation certificate did not contain Subject Alternative Name")?;

    for name in san.0 {
        if let GeneralName::UniformResourceIdentifier(uri) = name
            && uri.as_str() == expected_identity
        {
            return Ok(());
        }
    }

    anyhow::bail!(
        "Attestation workflow identity mismatch: expected {expected_identity}, but certificate did not contain it"
    );
}

fn verify_github_issuer(certificate: &Certificate) -> Result<()> {
    const LEGACY: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.4.1.57264.1.1");
    const V2: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.4.1.57264.1.8");
    const EXPECTED: &str = "https://token.actions.githubusercontent.com";
    let mut seen = std::collections::HashSet::new();
    for extension in certificate.tbs_certificate.extensions.iter().flatten() {
        if extension.extn_id != LEGACY && extension.extn_id != V2 {
            continue;
        }
        anyhow::ensure!(
            seen.insert(extension.extn_id),
            "Duplicate OIDC issuer extension"
        );
        let bytes = extension.extn_value.as_bytes();
        let issuer = if extension.extn_id == V2 {
            x509_cert::der::asn1::Utf8StringRef::from_der(bytes)?.as_str()
        } else {
            std::str::from_utf8(bytes)?
        };
        anyhow::ensure!(issuer == EXPECTED, "Attestation OIDC issuer mismatch");
    }
    anyhow::ensure!(
        !seen.is_empty(),
        "Attestation certificate missing OIDC issuer"
    );
    Ok(())
}

pub(crate) fn verify_payload_subject_digest(payload: &[u8], expected_digest: &str) -> Result<()> {
    let statement: Value =
        serde_json::from_slice(payload).context("Failed to parse attestation payload")?;
    anyhow::ensure!(
        statement["_type"] == "https://in-toto.io/Statement/v1"
            && statement["predicateType"] == "https://slsa.dev/provenance/v1"
            && statement["predicate"].is_object(),
        "Expected an in-toto SLSA v1 provenance statement"
    );
    let subjects = statement
        .get("subject")
        .and_then(Value::as_array)
        .context("Attestation payload did not include subject entries")?;

    let expected_digest = expected_digest
        .strip_prefix("sha256:")
        .unwrap_or(expected_digest);
    let found = subjects.iter().any(|subject| {
        subject
            .get("digest")
            .and_then(|digest| digest.get("sha256"))
            .and_then(Value::as_str)
            == Some(expected_digest)
    });

    if !found {
        anyhow::bail!("Attestation subject digest mismatch: expected sha256:{expected_digest}");
    }

    Ok(())
}

fn verify_dsse_signature(
    certificate: &Certificate,
    envelope: &DsseEnvelope,
    payload: &[u8],
) -> Result<()> {
    let verification_key =
        CosignVerificationKey::try_from(&certificate.tbs_certificate.subject_public_key_info)
            .context("Failed to extract attestation certificate public key")?;
    let signature = envelope
        .signatures
        .first()
        .context("Attestation DSSE envelope did not include a signature")?;

    let pae = dsse_pae(&envelope.payload_type, payload);
    verification_key
        .verify_signature(Signature::Base64Encoded(signature.sig.as_bytes()), &pae)
        .context("DSSE signature verification failed")?;

    Ok(())
}

pub(crate) fn dsse_pae(payload_type: &str, payload: &[u8]) -> Vec<u8> {
    let mut pae = Vec::new();
    pae.extend_from_slice(b"DSSEv1");
    pae.push(b' ');
    pae.extend_from_slice(payload_type.len().to_string().as_bytes());
    pae.push(b' ');
    pae.extend_from_slice(payload_type.as_bytes());
    pae.push(b' ');
    pae.extend_from_slice(payload.len().to_string().as_bytes());
    pae.push(b' ');
    pae.extend_from_slice(payload);
    pae
}

fn normalize_log_key_id(key_id: &str) -> Result<String> {
    let key_id = key_id.trim();
    if !key_id.is_empty()
        && key_id.len().is_multiple_of(2)
        && key_id.chars().all(|ch| ch.is_ascii_hexdigit())
    {
        return Ok(key_id.to_ascii_lowercase());
    }

    Ok(hex::encode(
        BASE64
            .decode(key_id)
            .context("Failed to decode Rekor logId.keyId")?,
    ))
}

fn parse_json_u64(value: &Value) -> Result<u64> {
    if let Some(n) = value.as_u64() {
        return Ok(n);
    }

    if let Some(s) = value.as_str() {
        return s
            .parse::<u64>()
            .context("Failed to parse JSON integer string");
    }

    anyhow::bail!("Expected JSON integer or integer string")
}

#[derive(Debug, Deserialize)]
struct AttestationsResponse {
    attestations: Vec<Attestation>,
}

#[derive(Debug, Deserialize)]
struct Attestation {
    bundle: Option<SigstoreBundle>,
    bundle_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SigstoreBundle {
    #[serde(rename = "mediaType")]
    media_type: Option<String>,
    #[serde(rename = "dsseEnvelope")]
    dsse_envelope: Option<DsseEnvelope>,
    #[serde(rename = "verificationMaterial")]
    verification_material: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub(crate) struct DsseEnvelope {
    payload: String,
    #[serde(rename = "payloadType")]
    payload_type: String,
    signatures: Vec<DsseSignature>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
struct DsseSignature {
    sig: String,
    #[serde(default)]
    keyid: String,
}

#[derive(Debug)]
struct ParsedBundle {
    require_proof: bool,
    envelope: DsseEnvelope,
    payload: Vec<u8>,
    certificate_chain: Vec<Vec<u8>>,
    tlog_entries: Vec<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounds_actual_json_body_and_blocks_untrusted_redirect_before_request() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
        let server = MockServer::start().await;
        Mock::given(path("/large"))
            .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(4096)))
            .mount(&server)
            .await;
        Mock::given(path("/redirect"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/forbidden", server.uri())),
            )
            .mount(&server)
            .await;
        Mock::given(path("/forbidden"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(bundle_redirect_policy())
            .build()
            .unwrap();
        let error = read_json_bounded::<Value>(
            client
                .get(format!("{}/large", server.uri()))
                .send()
                .await
                .unwrap(),
            64,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("size limit"));
        assert!(
            client
                .get(format!("{}/redirect", server.uri()))
                .send()
                .await
                .unwrap_err()
                .is_redirect()
        );
        server.verify().await;
        for rejected in [
            "http://github.com/a",
            "https://github.com:444/a",
            "https://github.com.attacker.example/a",
            "https://token@github.com/a",
        ] {
            assert!(!allowed_bundle_url(&rejected.parse().unwrap()));
        }
        assert!(allowed_bundle_url(
            &"https://objects.githubusercontent.com/a".parse().unwrap()
        ));
    }

    #[test]
    fn rejects_parseable_proof_hash_and_logged_body_substitution() {
        let (_, root, _) = fixture("0.71.0");
        for pointer in ["/rootHash", "/hashes/0"] {
            let modified = altered_bundle(|v| {
                let field = v["verificationMaterial"]["tlogEntries"][0]["inclusionProof"]
                    .pointer_mut(pointer)
                    .unwrap();
                let mut bytes = BASE64.decode(field.as_str().unwrap()).unwrap();
                assert_eq!(bytes.len(), 32);
                bytes[0] ^= 1;
                *field = BASE64.encode(bytes).into();
            });
            assert!(
                format!("{:#}", rekor::verify(&modified, &root).unwrap_err())
                    .contains("Merkle inclusion")
            );
        }
        for field in ["payloadHash", "envelopeHash"] {
            let modified = altered_bundle(|v| {
                let logged = &mut v["verificationMaterial"]["tlogEntries"][0]["canonicalizedBody"];
                let mut body: Value =
                    serde_json::from_slice(&BASE64.decode(logged.as_str().unwrap()).unwrap())
                        .unwrap();
                body["spec"][field]["value"] = "00".repeat(32).into();
                *logged = BASE64.encode(serde_json::to_vec(&body).unwrap()).into();
            });
            assert!(
                format!("{:#}", rekor::verify(&modified, &root).unwrap_err())
                    .contains("DSSE hash mismatch")
            );
        }
        for field in ["signature", "verifier"] {
            let modified = altered_bundle(|v| {
                let logged = &mut v["verificationMaterial"]["tlogEntries"][0]["canonicalizedBody"];
                let mut body: Value =
                    serde_json::from_slice(&BASE64.decode(logged.as_str().unwrap()).unwrap())
                        .unwrap();
                if field == "signature" {
                    let mut sig = BASE64
                        .decode(body["spec"]["signatures"][0][field].as_str().unwrap())
                        .unwrap();
                    *sig.last_mut().unwrap() ^= 1;
                    body["spec"]["signatures"][0][field] = BASE64.encode(sig).into();
                } else {
                    let (old, _, _) = fixture("0.40.0");
                    let old = parse_bundle(&old).unwrap();
                    let cert = Certificate::from_der(&old.certificate_chain[0]).unwrap();
                    use x509_cert::der::EncodePem;
                    body["spec"]["signatures"][0][field] = BASE64
                        .encode(cert.to_pem(x509_cert::der::pem::LineEnding::LF).unwrap())
                        .into();
                }
                *logged = BASE64.encode(serde_json::to_vec(&body).unwrap()).into();
            });
            let expected = if field == "signature" {
                "Logged DSSE signature mismatch"
            } else {
                "Logged certificate mismatch"
            };
            assert!(
                format!("{:#}", rekor::verify(&modified, &root).unwrap_err()).contains(expected)
            );
        }
        let modified = altered_bundle(|v| {
            v["verificationMaterial"]["tlogEntries"][0]["logId"]["keyId"] =
                BASE64.encode([0; 32]).into();
        });
        assert!(
            rekor::verify(&modified, &root)
                .unwrap_err()
                .to_string()
                .contains("Unknown Rekor")
        );
    }

    fn current_fixture_value() -> Value {
        serde_json::from_slice(include_bytes!(
            "../../tests/fixtures/sigstore/nyxid-v0.71.0.bundle.json"
        ))
        .unwrap()
    }

    fn altered_bundle(edit: impl FnOnce(&mut Value)) -> ParsedBundle {
        let mut value = current_fixture_value();
        edit(&mut value);
        parse_bundle(&Attestation {
            bundle: Some(serde_json::from_value(value).unwrap()),
            bundle_url: None,
        })
        .unwrap()
    }

    #[test]
    fn rejects_parseable_forged_set_time_index_and_signatures() {
        let (_, root, _) = fixture("0.71.0");
        for field in ["integratedTime", "logIndex"] {
            let modified = altered_bundle(|v| {
                let entry = &mut v["verificationMaterial"]["tlogEntries"][0];
                entry[field] = (parse_json_u64(&entry[field]).unwrap() + 1)
                    .to_string()
                    .into();
            });
            assert!(
                format!("{:#}", rekor::verify(&modified, &root).unwrap_err())
                    .contains("SET signature")
            );
        }
        let modified = altered_bundle(|v| {
            let sig = &mut v["verificationMaterial"]["tlogEntries"][0]["inclusionPromise"]["signedEntryTimestamp"];
            let mut bytes = BASE64.decode(sig.as_str().unwrap()).unwrap();
            *bytes.last_mut().unwrap() ^= 1;
            *sig = BASE64.encode(bytes).into();
        });
        assert!(
            format!("{:#}", rekor::verify(&modified, &root).unwrap_err()).contains("SET signature")
        );
    }

    #[test]
    fn rejects_parseable_checkpoint_signature_and_nonminimal_merkle_paths() {
        let (_, root, _) = fixture("0.71.0");
        let modified = altered_bundle(|v| {
            let checkpoint = &mut v["verificationMaterial"]["tlogEntries"][0]["inclusionProof"]["checkpoint"]
                ["envelope"];
            let original = checkpoint.as_str().unwrap();
            let (prefix, sig) = original.trim_end().rsplit_once(' ').unwrap();
            let mut bytes = BASE64.decode(sig).unwrap();
            *bytes.last_mut().unwrap() ^= 1;
            *checkpoint = format!("{prefix} {}\n", BASE64.encode(bytes)).into();
        });
        assert!(
            format!("{:#}", rekor::verify(&modified, &root).unwrap_err())
                .contains("Checkpoint signature")
        );
        for extra in [false, true] {
            let modified = altered_bundle(|v| {
                let hashes =
                    v["verificationMaterial"]["tlogEntries"][0]["inclusionProof"]["hashes"]
                        .as_array_mut()
                        .unwrap();
                if extra {
                    hashes.push(hashes[0].clone());
                } else {
                    hashes.pop();
                }
            });
            assert!(
                format!("{:#}", rekor::verify(&modified, &root).unwrap_err()).contains("Merkle")
            );
        }
    }

    #[test]
    fn issuer_and_statement_policy_are_explicit() {
        let (attestation, _, digest) = fixture("0.71.0");
        let parsed = parse_bundle(&attestation).unwrap();
        let cert = Certificate::from_der(&parsed.certificate_chain[0]).unwrap();
        verify_github_issuer(&cert).unwrap();
        let mut changed = cert.clone();
        changed
            .tbs_certificate
            .extensions
            .as_mut()
            .unwrap()
            .retain(|e| !e.extn_id.to_string().starts_with("1.3.6.1.4.1.57264.1."));
        assert!(verify_github_issuer(&changed).is_err());
        let mut changed = cert.clone();
        let issuer = changed
            .tbs_certificate
            .extensions
            .as_mut()
            .unwrap()
            .iter_mut()
            .find(|e| e.extn_id.to_string() == "1.3.6.1.4.1.57264.1.1")
            .unwrap();
        issuer.extn_value =
            x509_cert::der::asn1::OctetString::new(b"https://attacker.example").unwrap();
        assert!(verify_github_issuer(&changed).is_err());
        let mut payload: Value = serde_json::from_slice(&parsed.payload).unwrap();
        payload["predicateType"] = "https://attacker.example/statement".into();
        assert!(
            verify_payload_subject_digest(&serde_json::to_vec(&payload).unwrap(), digest).is_err()
        );
        for media in [
            None,
            Some("application/json"),
            Some("application/vnd.dev.sigstore.bundle.v0.4+json"),
        ] {
            let mut value = current_fixture_value();
            value["mediaType"] = media.into();
            assert!(
                parse_bundle(&Attestation {
                    bundle: Some(serde_json::from_value(value).unwrap()),
                    bundle_url: None
                })
                .is_err()
            );
        }
    }

    fn fixture(tag: &str) -> (Attestation, trust::Root, &'static str) {
        let (bytes, digest) = match tag {
            "0.15.0" => (
                include_bytes!("../../tests/fixtures/sigstore/nyxid-v0.15.0.bundle.json")
                    .as_slice(),
                "7476e161a62e71aeaf1a66ec5d65104a3b1e8e68af2aecf264448d7cde27ee31",
            ),
            "0.40.0" => (
                include_bytes!("../../tests/fixtures/sigstore/nyxid-v0.40.0.bundle.json")
                    .as_slice(),
                "b89a8627119c803cbcf40c0c633e01c6e0bda7084afd51476f784b7cd0a820da",
            ),
            _ => (
                include_bytes!("../../tests/fixtures/sigstore/nyxid-v0.71.0.bundle.json")
                    .as_slice(),
                "9b068f9d9361f460987c447d56e10ad1c668821c19b1e23a184c964eebed1f77",
            ),
        };
        (
            Attestation {
                bundle: Some(serde_json::from_slice(bytes).unwrap()),
                bundle_url: None,
            },
            trust::Root::from_authenticated_json(include_bytes!(
                "../../tests/fixtures/sigstore/trusted_root.json"
            ))
            .unwrap(),
            digest,
        )
    }

    #[test]
    fn verifies_published_historical_and_current_workflow_bundles() {
        for tag in ["0.15.0", "0.40.0", "0.71.0"] {
            let (attestation, root, digest) = fixture(tag);
            let identity = format!(
                "https://github.com/ChronoAIProject/NyxID/.github/workflows/release.yml@refs/tags/v{tag}"
            );
            verify_single_attestation(&attestation, &root, digest, &identity).unwrap();
            assert!(verify_single_attestation(&attestation, &root, "0000", &identity).is_err());
            assert!(verify_single_attestation(&attestation, &root, digest, "https://github.com/attacker/repo/.github/workflows/release.yml@refs/tags/v0.71.0").is_err());
            assert!(
                verify_single_attestation(
                    &attestation,
                    &root,
                    digest,
                    &identity.replace("refs/tags", "refs/heads")
                )
                .is_err()
            );
        }
    }

    #[test]
    fn verifies_published_machine_image_workflow_bundle() {
        let attestation = Attestation {
            bundle: Some(
                serde_json::from_slice(include_bytes!(
                    "../../tests/fixtures/sigstore/nyxid-node-machine-v0.71.0.bundle.json"
                ))
                .unwrap(),
            ),
            bundle_url: None,
        };
        let (_, root, _) = fixture("0.71.0");
        let digest = "e634854db76b1f7230e79312bab23cb394831a4909920f8a93890878bffe0c1d";
        let identity = "https://github.com/ChronoAIProject/NyxID/.github/workflows/publish-images.yml@refs/tags/v0.71.0";
        verify_single_attestation(&attestation, &root, digest, identity).unwrap();
        assert!(
            verify_single_attestation(
                &attestation,
                &root,
                digest,
                &identity.replace("publish-images", "release")
            )
            .is_err()
        );
        assert!(verify_single_attestation(&attestation, &root, &"0".repeat(64), identity).is_err());
    }

    #[test]
    fn rejects_tampered_log_evidence_and_unauthenticated_time() {
        let (attestation, root, digest) = fixture("0.71.0");
        let identity = "https://github.com/ChronoAIProject/NyxID/.github/workflows/release.yml@refs/tags/v0.71.0";
        for pointer in [
            "/integratedTime",
            "/logIndex",
            "/logId/keyId",
            "/inclusionPromise/signedEntryTimestamp",
            "/inclusionProof/rootHash",
            "/inclusionProof/treeSize",
            "/inclusionProof/logIndex",
            "/inclusionProof/hashes/0",
            "/inclusionProof/checkpoint/envelope",
            "/canonicalizedBody",
        ] {
            let mut value: Value = serde_json::from_slice(include_bytes!(
                "../../tests/fixtures/sigstore/nyxid-v0.71.0.bundle.json"
            ))
            .unwrap();
            let entry = &mut value["verificationMaterial"]["tlogEntries"][0];
            *entry.pointer_mut(pointer).unwrap() = Value::String("0".into());
            let modified = Attestation {
                bundle: Some(serde_json::from_value(value).unwrap()),
                bundle_url: None,
            };
            assert!(
                verify_single_attestation(&modified, &root, digest, identity).is_err(),
                "accepted {pointer}"
            );
        }
        let mut bundle = parse_bundle(&attestation).unwrap();
        // Proof does not sign the outer time. No SET means no trusted time,
        // even in an older promise-optional-looking shape.
        bundle.tlog_entries[0]
            .as_object_mut()
            .unwrap()
            .remove("inclusionPromise");
        assert!(rekor::verify(&bundle, &root).is_err());
    }

    #[test]
    fn rejects_log_material_substitution_and_enforces_format_policy() {
        let (attestation, root, _) = fixture("0.71.0");
        let mut bundle = parse_bundle(&attestation).unwrap();
        let original = bundle.envelope.signatures[0].sig.clone();
        bundle.envelope.signatures[0].sig = BASE64.encode(b"attacker");
        assert!(rekor::verify(&bundle, &root).is_err());
        bundle.envelope.signatures[0].sig = original;
        bundle.certificate_chain[0][10] ^= 1;
        assert!(rekor::verify(&bundle, &root).is_err());

        let mut bundle = parse_bundle(&attestation).unwrap();
        bundle.tlog_entries[0]
            .as_object_mut()
            .unwrap()
            .remove("inclusionProof");
        assert!(rekor::verify(&bundle, &root).is_err());
        bundle.require_proof = false;
        rekor::verify(&bundle, &root).unwrap();
        bundle.tlog_entries[0]["inclusionProof"] = serde_json::json!({});
        assert!(rekor::verify(&bundle, &root).is_err());
    }

    #[test]
    fn verifies_fixture_subject_digest() {
        let payload =
            include_bytes!("../../tests/fixtures/update-attestation-statement.json").as_slice();
        verify_payload_subject_digest(
            payload,
            "8c5b8a213a6d3d0c74a1f3a1c9dbd9ed93094b2b2ca8c7a4d00365bd7a9a6a6b",
        )
        .unwrap();
        verify_payload_subject_digest(
            payload,
            "sha256:8c5b8a213a6d3d0c74a1f3a1c9dbd9ed93094b2b2ca8c7a4d00365bd7a9a6a6b",
        )
        .unwrap();
        assert!(verify_payload_subject_digest(payload, "0000").is_err());
    }

    #[test]
    fn creates_dsse_pae() {
        assert_eq!(
            dsse_pae("application/vnd.in-toto+json", b"hello"),
            b"DSSEv1 28 application/vnd.in-toto+json 5 hello"
        );
    }

    #[test]
    fn normalizes_rekor_log_key_ids() {
        assert_eq!(normalize_log_key_id("ABcd42").unwrap(), "abcd42");
        assert_eq!(normalize_log_key_id("q80=").unwrap(), "abcd");
    }

    #[test]
    fn parses_json_u64_from_number_or_string() {
        assert_eq!(parse_json_u64(&Value::from(42)).unwrap(), 42);
        assert_eq!(parse_json_u64(&Value::from("42")).unwrap(), 42);
        assert!(parse_json_u64(&Value::from("nope")).is_err());
    }

    #[test]
    fn parse_json_u64_rejects_non_integer_json_values() {
        assert!(parse_json_u64(&Value::Bool(true)).is_err());
        assert!(parse_json_u64(&serde_json::json!({ "n": 42 })).is_err());
    }

    #[test]
    fn extracts_single_certificate_from_raw_bytes() {
        let material = serde_json::json!({
            "certificate": {
                "rawBytes": BASE64.encode(b"leaf-cert")
            }
        });

        assert_eq!(
            extract_certificate_chain(&material).unwrap(),
            vec![b"leaf-cert".to_vec()]
        );
    }

    #[test]
    fn extracts_certificate_chain_array_in_order() {
        let material = serde_json::json!({
            "x509CertificateChain": {
                "certificates": [
                    {"rawBytes": BASE64.encode(b"leaf")},
                    {"rawBytes": BASE64.encode(b"intermediate")}
                ]
            }
        });

        assert_eq!(
            extract_certificate_chain(&material).unwrap(),
            vec![b"leaf".to_vec(), b"intermediate".to_vec()]
        );
    }

    #[test]
    fn extract_certificate_chain_rejects_missing_or_malformed_material() {
        assert!(extract_certificate_chain(&serde_json::json!({})).is_err());

        let malformed = serde_json::json!({
            "x509CertificateChain": {
                "certificates": [
                    {"rawBytes": "***not-base64***"}
                ]
            }
        });
        let err = extract_certificate_chain(&malformed).expect_err("malformed chain");
        assert!(err.to_string().contains("Failed to decode"));
    }

    #[test]
    fn extracts_tlog_entries_from_array_or_rekor_bundle() {
        let material = serde_json::json!({
            "tlogEntries": [
                {"logIndex": 7}
            ]
        });
        assert_eq!(extract_tlog_entries(&material).unwrap()[0]["logIndex"], 7);

        let legacy = serde_json::json!({
            "rekorBundle": {"logIndex": 9}
        });
        assert_eq!(extract_tlog_entries(&legacy).unwrap()[0]["logIndex"], 9);

        assert!(extract_tlog_entries(&serde_json::json!({})).is_err());
    }

    #[test]
    fn parse_bundle_extracts_payload_certificate_chain_and_tlog_entries() {
        let payload = br#"{"subject":[]}"#;
        let attestation = Attestation {
            bundle_url: None,
            bundle: Some(SigstoreBundle {
                media_type: Some("application/vnd.dev.sigstore.bundle.v0.3+json".into()),
                dsse_envelope: Some(DsseEnvelope {
                    payload: BASE64.encode(payload),
                    payload_type: "application/vnd.in-toto+json".to_string(),
                    signatures: vec![DsseSignature {
                        sig: BASE64.encode(b"sig"),
                        keyid: String::new(),
                    }],
                }),
                verification_material: Some(serde_json::json!({
                    "certificate": {
                        "rawBytes": BASE64.encode(b"leaf-cert")
                    },
                    "tlogEntries": [
                        {"logIndex": 1}
                    ]
                })),
            }),
        };

        let parsed = parse_bundle(&attestation).expect("parsed bundle");
        assert_eq!(parsed.payload, payload);
        assert_eq!(parsed.envelope.payload_type, "application/vnd.in-toto+json");
        assert_eq!(parsed.envelope.signatures[0].sig, BASE64.encode(b"sig"));
        assert_eq!(parsed.certificate_chain, vec![b"leaf-cert".to_vec()]);
        assert_eq!(parsed.tlog_entries[0]["logIndex"], 1);
        assert!(parsed.envelope.payload.len() > parsed.payload.len());
        assert!(attestation.bundle_url.is_none());
    }

    #[test]
    fn parse_bundle_rejects_missing_bundle_sections() {
        let missing_bundle = Attestation {
            bundle: None,
            bundle_url: None,
        };
        assert!(parse_bundle(&missing_bundle).is_err());

        let missing_envelope = Attestation {
            bundle_url: None,
            bundle: Some(SigstoreBundle {
                media_type: Some("application/vnd.dev.sigstore.bundle.v0.3+json".into()),
                dsse_envelope: None,
                verification_material: Some(serde_json::json!({})),
            }),
        };
        let err = parse_bundle(&missing_envelope).expect_err("missing envelope");
        assert!(err.to_string().contains("DSSE envelope"));
    }

    #[test]
    fn writes_big_endian_u16_and_u24_lengths() {
        let mut out = Vec::new();
        write_u16(&mut out, 0x1234).unwrap();
        write_u24(&mut out, 0x00ab_cdef).unwrap();
        assert_eq!(out, vec![0x12, 0x34, 0xab, 0xcd, 0xef]);

        assert!(write_u16(&mut Vec::new(), 0x1_0000).is_err());
        assert!(write_u24(&mut Vec::new(), 0x0100_0000).is_err());
    }

    #[test]
    fn normalize_log_key_id_trims_hex_and_rejects_invalid_base64() {
        assert_eq!(normalize_log_key_id("  ABcd42  ").unwrap(), "abcd42");
        assert!(normalize_log_key_id("not valid base64!").is_err());
    }
}
