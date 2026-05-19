//! AWS Signature V4 request signing.
//!
//! Wraps the upstream `aws-sigv4` crate so the rest of NyxID consumes a
//! stable, narrow API: parse the JSON credential blob the user stored,
//! produce the headers required to make a request signed, return them
//! as plain `(name, value)` pairs.
//!
//! Why upstream and not hand-rolled: `aws-sigv4` bakes in canonical-URI
//! path normalization (`./` and `../` collapse), RFC 3986 query
//! encoding, header value whitespace normalization, duplicate header
//! aggregation, host-port handling, and the `UNSIGNED-PAYLOAD` /
//! `STREAMING-...` payload-marker variants — every one of which is a
//! latent correctness bug in a hand-rolled signer.

use std::time::SystemTime;

use aws_credential_types::Credentials;
use aws_sigv4::http_request::{
    PayloadChecksumKind, SignableBody, SignableRequest, SignatureLocation, SigningSettings, sign,
};
use aws_sigv4::sign::v4;
use serde::{Deserialize, Serialize};

use crate::error::{CloudAuthError, CloudAuthResult};

/// Decoded `aws_sigv4` credential payload.
///
/// Stored on `UserApiKey.credential_encrypted` as a JSON object. The
/// `region` and `service` fields are required and must match the
/// target endpoint — for AWS Cost Explorer that's `us-east-1` + `ce`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub region: String,
    pub service: String,
    /// Optional STS session token. When present, the signer attaches
    /// it as `X-Amz-Security-Token` and includes it in the signed-
    /// headers list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_token: Option<String>,
}

impl AwsCredentials {
    pub fn from_json(raw: &str) -> CloudAuthResult<Self> {
        let trimmed = raw.trim();
        let creds: Self = serde_json::from_str(trimmed).map_err(|e| {
            CloudAuthError::InvalidCredential(format!(
                "aws_sigv4 credential must be a JSON object with access_key_id, secret_access_key, region, service: {}",
                e
            ))
        })?;
        if creds.access_key_id.is_empty()
            || creds.secret_access_key.is_empty()
            || creds.region.is_empty()
            || creds.service.is_empty()
        {
            return Err(CloudAuthError::InvalidCredential(
                "aws_sigv4 credential is missing one of access_key_id, secret_access_key, region, service".to_string(),
            ));
        }
        Ok(creds)
    }
}

/// A header name/value pair to attach to the outgoing request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedHeader {
    pub name: String,
    pub value: String,
}

/// Sign `method` + `url` + `headers` + `body` under SigV4 with `creds`.
///
/// Returns only the *additional* headers the caller needs to attach to
/// make the request signed (`Authorization`, `X-Amz-Date`,
/// `X-Amz-Content-Sha256`, optional `X-Amz-Security-Token`). The
/// caller must not pre-attach any of those — the upstream signer
/// canonicalizes over the provided header set and then we layer the
/// produced headers on top.
///
/// `headers` must reflect every other header that will be on the wire
/// at send time (Content-Type, X-Amz-Target, etc.). Headers attached
/// after this returns are NOT signed and will cause AWS to reject the
/// request with a signature mismatch.
pub fn sign_request(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: &[u8],
    creds: &AwsCredentials,
) -> CloudAuthResult<Vec<SignedHeader>> {
    let aws_creds = Credentials::new(
        &creds.access_key_id,
        &creds.secret_access_key,
        creds.session_token.clone(),
        None,
        "nyxid-cloud-auth",
    );
    let identity = aws_creds.into();
    let signing_params = v4::SigningParams::builder()
        .identity(&identity)
        .region(&creds.region)
        .name(&creds.service)
        .time(SystemTime::now())
        .settings({
            let mut settings = SigningSettings::default();
            // Cost Explorer is JSON-RPC POST `/`; the default request
            // form is plain-old SigV4, not query-string presigning.
            settings.signature_location = SignatureLocation::Headers;
            // Always include the payload hash header so the server
            // can integrity-check the body. `aws-sigv4` defaults to
            // "include for sigv4" anyway, but be explicit so a future
            // settings tweak doesn't silently change behavior.
            settings.payload_checksum_kind = PayloadChecksumKind::XAmzSha256;
            settings
        })
        .build()
        .map_err(|e| CloudAuthError::Signing(format!("SigningParams build: {e}")))?;
    let signing_params = aws_sigv4::http_request::SigningParams::from(signing_params);

    // Borrow the caller's headers as `(&str, &str)` for the upstream
    // SignableRequest constructor.
    let header_refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(n, v)| (n.as_str(), v.as_str()))
        .collect();
    let signable = SignableRequest::new(
        method,
        url,
        header_refs.iter().copied(),
        SignableBody::Bytes(body),
    )
    .map_err(|e| CloudAuthError::Signing(format!("SignableRequest: {e}")))?;

    let signing_output = sign(signable, &signing_params)
        .map_err(|e| CloudAuthError::Signing(format!("sign: {e}")))?;
    let (instructions, _signature) = signing_output.into_parts();

    // `SigningInstructions` exposes its applied headers and query
    // params via `apply_to_request_http1x` (which mutates an
    // `http::Request`). We want a plain header list to hand back to
    // the caller's reqwest builder, so go through a stub
    // `http::Request<()>` and read the headers off.
    let mut stub = http::Request::builder()
        .method(method)
        .uri(url)
        .body(())
        .map_err(|e| CloudAuthError::Signing(format!("stub http::Request: {e}")))?;
    instructions.apply_to_request_http1x(&mut stub);

    let mut out = Vec::with_capacity(4);
    for (name, value) in stub.headers().iter() {
        // Skip the host header — reqwest derives it from the URL and
        // forwarding our copy would create a duplicate. The upstream
        // signer already included it in the canonical request, which
        // is what matters for the signature.
        if name.as_str().eq_ignore_ascii_case("host") {
            continue;
        }
        let value_str = match value.to_str() {
            Ok(s) => s,
            Err(_) => {
                return Err(CloudAuthError::Signing(format!(
                    "signed header {name} contains non-ASCII bytes"
                )));
            }
        };
        out.push(SignedHeader {
            name: name.as_str().to_string(),
            value: value_str.to_string(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_malformed_credential_json() {
        let err = AwsCredentials::from_json("not json").unwrap_err();
        assert!(matches!(err, CloudAuthError::InvalidCredential(_)));
    }

    #[test]
    fn rejects_credential_missing_fields() {
        let err = AwsCredentials::from_json(r#"{"access_key_id": "k", "secret_access_key": "s"}"#)
            .unwrap_err();
        assert!(matches!(err, CloudAuthError::InvalidCredential(_)));
    }

    /// End-to-end: feed real-looking credentials to `aws-sigv4` and
    /// confirm we get back the canonical four-header set (Authorization,
    /// X-Amz-Date, X-Amz-Content-Sha256, plus X-Amz-Security-Token when
    /// applicable). The actual signature math is upstream's job and is
    /// covered by their tests — we just verify our shim emits the
    /// expected shape.
    #[test]
    fn returns_expected_signed_headers() {
        let creds = AwsCredentials {
            access_key_id: "AKIDEXAMPLE".to_string(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_string(),
            region: "us-east-1".to_string(),
            service: "ce".to_string(),
            session_token: None,
        };
        let signed = sign_request(
            "POST",
            "https://ce.us-east-1.amazonaws.com/",
            &[(
                "content-type".to_string(),
                "application/x-amz-json-1.1".to_string(),
            )],
            b"",
            &creds,
        )
        .expect("sign");

        let names: Vec<&str> = signed.iter().map(|h| h.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert!(
            sorted
                .iter()
                .any(|n| n.eq_ignore_ascii_case("authorization")),
            "missing authorization header: {sorted:?}"
        );
        assert!(
            sorted.iter().any(|n| n.eq_ignore_ascii_case("x-amz-date")),
            "missing x-amz-date: {sorted:?}"
        );
        let auth = signed
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("authorization"))
            .unwrap();
        assert!(auth.value.starts_with("AWS4-HMAC-SHA256 "));
        assert!(auth.value.contains("Credential=AKIDEXAMPLE/"));
        assert!(auth.value.contains("/us-east-1/ce/aws4_request"));
        // No session-token header when the credential omits it.
        assert!(
            !signed
                .iter()
                .any(|h| h.name.eq_ignore_ascii_case("x-amz-security-token"))
        );
    }

    #[test]
    fn session_token_propagates_to_signed_header_set() {
        let creds = AwsCredentials {
            access_key_id: "AKIDEXAMPLE".to_string(),
            secret_access_key: "secret".to_string(),
            region: "us-east-1".to_string(),
            service: "ce".to_string(),
            session_token: Some("FQoGZ...short".to_string()),
        };
        let signed = sign_request(
            "POST",
            "https://ce.us-east-1.amazonaws.com/",
            &[],
            b"{}",
            &creds,
        )
        .expect("sign");
        let token_header = signed
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("x-amz-security-token"))
            .expect("expected session token to appear as a signed header");
        assert_eq!(token_header.value, "FQoGZ...short");
    }
}
