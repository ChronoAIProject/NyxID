use std::time::Duration;

use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use const_oid::db::{rfc5280::ID_KP_CODE_SIGNING, rfc6962::CT_PRECERT_SCTS};
use pki_types::{CertificateDer, UnixTime};
use reqwest::header::CONTENT_TYPE;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sigstore::crypto::{CosignVerificationKey, Signature};
use sigstore::trust::{TrustRoot, sigstore::SigstoreTrustRoot};
use webpki::{EndEntityCert, KeyUsage};
use x509_cert::Certificate;
use x509_cert::der::{Decode, Encode};
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::ext::pkix::sct::{HashAlgorithm, SignatureAlgorithm, SignedCertificateTimestamp};
use x509_cert::ext::pkix::{SignedCertificateTimestampList, SubjectAltName};

const GITHUB_API_URL: &str = "https://api.github.com";
const MAX_ATTESTATIONS: usize = 30;

pub(crate) async fn verify_release_attestation(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    expected_digest: &str,
    expected_identity: &str,
) -> Result<()> {
    let attestations = fetch_github_attestations(client, owner, repo, expected_digest)
        .await
        .with_context(|| {
            format!("Failed to fetch GitHub attestations for sha256:{expected_digest}")
        })?;

    if attestations.is_empty() {
        anyhow::bail!("No GitHub artifact attestations found for sha256:{expected_digest}");
    }

    let trust_root = SigstoreTrustRoot::new(None)
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

async fn fetch_github_attestations(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    digest: &str,
) -> Result<Vec<Attestation>> {
    let url = format!("{GITHUB_API_URL}/repos/{owner}/{repo}/attestations/sha256:{digest}");
    let response = client
        .get(&url)
        .query(&[("per_page", MAX_ATTESTATIONS.to_string())])
        .send()
        .await
        .with_context(|| format!("Failed to query GitHub attestation API: {url}"))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new());
    }

    let response = response
        .error_for_status()
        .with_context(|| format!("GitHub attestation API returned an error for {url}"))?;
    let body = response
        .json::<AttestationsResponse>()
        .await
        .context("Failed to parse GitHub attestation response")?;

    let mut attestations = Vec::new();
    for attestation in body.attestations {
        if attestation.bundle.is_some() {
            attestations.push(attestation);
        } else if let Some(bundle_url) = &attestation.bundle_url {
            let bundle = fetch_bundle_url(client, bundle_url).await?;
            attestations.push(Attestation {
                bundle: Some(bundle),
                bundle_url: attestation.bundle_url.clone(),
            });
        }
    }

    Ok(attestations)
}

async fn fetch_bundle_url(client: &reqwest::Client, bundle_url: &str) -> Result<SigstoreBundle> {
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

    response
        .json::<SigstoreBundle>()
        .await
        .context("Failed to parse downloaded attestation bundle")
}

fn verify_single_attestation(
    attestation: &Attestation,
    trust_root: &SigstoreTrustRoot,
    expected_digest: &str,
    expected_identity: &str,
) -> Result<()> {
    let parsed = parse_bundle(attestation)?;
    let signing_time = verify_rekor_entry(&parsed.tlog_entries, trust_root)?;
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
    let envelope = bundle
        .dsse_envelope
        .clone()
        .context("Attestation bundle did not include a DSSE envelope")?;
    let payload = BASE64
        .decode(&envelope.payload)
        .context("Failed to decode DSSE payload")?;
    let verification_material = bundle
        .verification_material
        .as_ref()
        .context("Attestation bundle did not include verification material")?;
    let certificate_chain = extract_certificate_chain(verification_material)?;
    let tlog_entries = extract_tlog_entries(verification_material)?;

    Ok(ParsedBundle {
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
        let mut chain = Vec::new();
        for certificate in certificates {
            let raw_bytes = certificate
                .get("rawBytes")
                .and_then(Value::as_str)
                .context("Certificate chain entry missing rawBytes")?;
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
        return Ok(entries.clone());
    }

    if let Some(entry) = verification_material.get("rekorBundle") {
        return Ok(vec![entry.clone()]);
    }

    anyhow::bail!("Attestation verification material did not include a Rekor transparency entry")
}

fn verify_certificate_chain(
    certificate_chain: &[Vec<u8>],
    trust_root: &SigstoreTrustRoot,
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
        .fulcio_certs()
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
    trust_root: &SigstoreTrustRoot,
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
    let ctfe_keys = trust_root
        .ctfe_keys()
        .context("Failed to load CTFE keys from Sigstore trust root")?;
    let log_key = ctfe_keys
        .get(&log_key_id)
        .with_context(|| format!("Signing certificate SCT used unknown CT log: {log_key_id}"))?;
    let issuer_spki = verified_path_issuer_spki(verified_path)?;
    let issuer_key_hash: [u8; 32] = Sha256::digest(&issuer_spki).into();
    let signed_payload = build_sct_signed_payload(certificate, &sct, &issuer_key_hash)?;

    verify_sct_algorithm(&sct)?;
    CosignVerificationKey::try_from_der(log_key)
        .context("Failed to parse Sigstore CT log public key")?
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

fn verify_sct_algorithm(sct: &SignedCertificateTimestamp) -> Result<()> {
    match (
        &sct.signature.algorithm.hash,
        &sct.signature.algorithm.signature,
    ) {
        (HashAlgorithm::Sha256 | HashAlgorithm::Sha384, SignatureAlgorithm::Ecdsa)
        | (
            HashAlgorithm::Sha256 | HashAlgorithm::Sha384 | HashAlgorithm::Sha512,
            SignatureAlgorithm::Rsa,
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

pub(crate) fn verify_payload_subject_digest(payload: &[u8], expected_digest: &str) -> Result<()> {
    let statement: Value =
        serde_json::from_slice(payload).context("Failed to parse attestation payload")?;
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

fn verify_rekor_entry(tlog_entries: &[Value], trust_root: &SigstoreTrustRoot) -> Result<UnixTime> {
    let entry = tlog_entries
        .first()
        .context("Attestation did not include a Rekor transparency entry")?;
    let tlog_entry: TransparencyLogEntry = serde_json::from_value(entry.clone())
        .context("Failed to parse Rekor transparency entry")?;

    let log_key_id = tlog_entry
        .log_id
        .as_ref()
        .and_then(|log_id| log_id.key_id.as_deref())
        .context("Rekor transparency entry missing logId.keyId")?;
    let log_key_id = normalize_log_key_id(log_key_id)?;
    let rekor_keys = trust_root
        .rekor_keys()
        .context("Failed to load Rekor keys from Sigstore trust root")?;
    if !rekor_keys.contains_key(&log_key_id) {
        anyhow::bail!("Rekor transparency entry was signed by an unknown log: {log_key_id}");
    }

    if tlog_entry.inclusion_promise.is_none() && tlog_entry.inclusion_proof.is_none() {
        anyhow::bail!("Rekor transparency entry did not include an inclusion promise or proof");
    }

    let canonicalized_body = tlog_entry
        .canonicalized_body
        .as_deref()
        .context("Rekor transparency entry missing canonicalizedBody")?;
    let canonicalized_body = decode_json_bytes(canonicalized_body)
        .context("Failed to decode Rekor canonicalized body")?;
    serde_json::from_slice::<Value>(&canonicalized_body)
        .context("Rekor canonicalized body was not valid JSON")?;

    let integrated_time = tlog_entry
        .integrated_time
        .as_ref()
        .context("Rekor transparency entry missing integratedTime")
        .and_then(parse_json_u64)?;

    Ok(UnixTime::since_unix_epoch(Duration::from_secs(
        integrated_time,
    )))
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

fn decode_json_bytes(value: &str) -> Result<Vec<u8>> {
    match BASE64.decode(value) {
        Ok(decoded) => Ok(decoded),
        Err(_) => Ok(value.as_bytes().to_vec()),
    }
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
    #[serde(rename = "dsseEnvelope")]
    dsse_envelope: Option<DsseEnvelope>,
    #[serde(rename = "verificationMaterial")]
    verification_material: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DsseEnvelope {
    payload: String,
    #[serde(rename = "payloadType")]
    payload_type: String,
    signatures: Vec<DsseSignature>,
}

#[derive(Debug, Clone, Deserialize)]
struct DsseSignature {
    sig: String,
}

#[derive(Debug)]
struct ParsedBundle {
    envelope: DsseEnvelope,
    payload: Vec<u8>,
    certificate_chain: Vec<Vec<u8>>,
    tlog_entries: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct TransparencyLogEntry {
    #[serde(rename = "logId")]
    log_id: Option<TransparencyLogId>,
    #[serde(rename = "integratedTime")]
    integrated_time: Option<Value>,
    #[serde(rename = "canonicalizedBody")]
    canonicalized_body: Option<String>,
    #[serde(rename = "inclusionPromise")]
    inclusion_promise: Option<Value>,
    #[serde(rename = "inclusionProof")]
    inclusion_proof: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct TransparencyLogId {
    #[serde(rename = "keyId")]
    key_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn decodes_json_bytes_from_base64_or_plain_json_text() {
        let encoded = BASE64.encode(br#"{"ok":true}"#);
        assert_eq!(decode_json_bytes(&encoded).unwrap(), br#"{"ok":true}"#);
        assert_eq!(
            decode_json_bytes(r#"{"ok":true}"#).unwrap(),
            br#"{"ok":true}"#
        );
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
                dsse_envelope: Some(DsseEnvelope {
                    payload: BASE64.encode(payload),
                    payload_type: "application/vnd.in-toto+json".to_string(),
                    signatures: vec![DsseSignature {
                        sig: BASE64.encode(b"sig"),
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
