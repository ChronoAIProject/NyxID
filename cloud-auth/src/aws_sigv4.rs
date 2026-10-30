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
#[derive(Clone, Deserialize, Serialize)]
pub struct AwsCredentials {
    pub access_key_id: zeroize::Zeroizing<String>,
    pub secret_access_key: zeroize::Zeroizing<String>,
    pub region: String,
    pub service: String,
    /// Optional STS session token. When present, the signer attaches
    /// it as `X-Amz-Security-Token` and includes it in the signed-
    /// headers list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_token: Option<zeroize::Zeroizing<String>>,
}

impl std::fmt::Debug for AwsCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AwsCredentials")
            .field("access_key_id", &"[REDACTED]")
            .field("secret_access_key", &"[REDACTED]")
            .field("region", &"[REDACTED]")
            .field("service", &"[REDACTED]")
            .field("session_token", &"[REDACTED]")
            .finish()
    }
}

impl AwsCredentials {
    pub fn from_json(raw: &str) -> CloudAuthResult<Self> {
        let trimmed = raw.trim();
        let creds: Self = serde_json::from_str(trimmed).map_err(|_| {
            CloudAuthError::InvalidCredential(
                "aws_sigv4 credential must be a JSON object with string access_key_id, secret_access_key, region, service and optional session_token",
            )
        })?;
        if creds.access_key_id.is_empty()
            || creds.secret_access_key.is_empty()
            || creds.region.is_empty()
            || creds.service.is_empty()
        {
            return Err(CloudAuthError::InvalidCredential("missing_required_fields"));
        }
        Ok(creds)
    }
}

/// A header name/value pair to attach to the outgoing request.
#[derive(Clone, PartialEq, Eq)]
pub struct SignedHeader {
    pub name: String,
    pub value: zeroize::Zeroizing<String>,
}

impl std::fmt::Debug for SignedHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignedHeader")
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .finish()
    }
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
        creds.access_key_id.as_str(),
        creds.secret_access_key.as_str(),
        creds.session_token.as_ref().map(|token| token.to_string()),
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
        .map_err(|_| CloudAuthError::Signing("invalid_signing_parameters"))?;
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
    .map_err(|_| CloudAuthError::Signing("invalid_signable_request"))?;

    let signing_output =
        sign(signable, &signing_params).map_err(|_| CloudAuthError::Signing("signing_failed"))?;
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
        .map_err(|_| CloudAuthError::Signing("invalid_http_request"))?;
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
                return Err(CloudAuthError::Signing("invalid_signed_header"));
            }
        };
        out.push(SignedHeader {
            name: name.as_str().to_string(),
            value: value_str.to_owned().into(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_credentials_and_signing_errors_have_fixed_diagnostics() {
        for raw in [
            r#"{"access_key_id":"distinctive-id","secret_access_key":"distinctive-secret","region":42}"#,
            r#"{"access_key_id":"distinctive-id","secret_access_key":"distinctive-secret","session_token":"distinctive-session","service":[]}"#,
        ] {
            let error = AwsCredentials::from_json(raw).unwrap_err();
            for output in [error.to_string(), format!("{error:?}")] {
                assert!(!output.contains("distinctive"));
            }
        }
        let credentials = AwsCredentials {
            access_key_id: "distinctive-id".to_owned().into(),
            secret_access_key: "distinctive-secret".to_owned().into(),
            session_token: Some("distinctive-session".to_owned().into()),
            region: "us-east-1".into(),
            service: "ce".into(),
        };
        let error = sign_request(
            "invalid method",
            "https://example.invalid/distinctive-token",
            &[],
            b"",
            &credentials,
        )
        .unwrap_err();
        for output in [error.to_string(), format!("{error:?}")] {
            assert!(!output.contains("distinctive"));
            assert!(!output.contains("example.invalid"));
        }
    }

    #[test]
    fn signed_header_debug_redacts_all_values() {
        let headers = vec![
            SignedHeader {
                name: "Authorization".into(),
                value: "AWS4-HMAC-SHA256 Credential=distinctive-signing-key-id"
                    .to_owned()
                    .into(),
            },
            SignedHeader {
                name: "X-Amz-Security-Token".into(),
                value: "distinctive-sts-session-token".to_owned().into(),
            },
            SignedHeader {
                name: "custom".into(),
                value: "distinctive-custom-header".to_owned().into(),
            },
        ];
        for output in [format!("{headers:?}"), format!("{headers:#?}")] {
            assert!(!output.contains("distinctive-"));
            assert!(output.contains("Authorization"));
            assert!(output.contains("[REDACTED]"));
        }
    }

    #[test]
    fn invalid_credential_errors_do_not_echo_supplied_values() {
        for raw in [
            r#"{"access_key_id":"id","secret_access_key":["distinctive-secret"],"region":"r","service":"s"}"#,
            r#"{"access_key_id":"id","secret_access_key":"secret","region":"r","service":"s","session_token":{"distinctive-secret":1}}"#,
            r#""distinctive-secret""#,
        ] {
            let error = AwsCredentials::from_json(raw).unwrap_err();
            assert!(!format!("{error}").contains("distinctive-secret"));
            assert!(!format!("{error:?}").contains("distinctive-secret"));
        }
    }

    #[test]
    fn credentials_debug_redacts_secrets_and_key_identifier() {
        let credentials = AwsCredentials {
            access_key_id: "distinctive-access-key-id".to_owned().into(),
            secret_access_key: "distinctive-secret-access-key".to_owned().into(),
            session_token: Some("distinctive-session-token".to_owned().into()),
            region: "us-east-1".into(),
            service: "ce".into(),
        };
        for output in [format!("{credentials:?}"), format!("{credentials:#?}")] {
            for secret in [
                &credentials.access_key_id,
                &credentials.secret_access_key,
                credentials.session_token.as_ref().unwrap(),
            ] {
                assert!(!output.contains(secret.as_str()));
            }
            assert!(output.contains("[REDACTED]"));
        }
        let json = serde_json::to_string(&credentials).unwrap();
        let decoded = AwsCredentials::from_json(&json).unwrap();
        assert_eq!(decoded.secret_access_key, credentials.secret_access_key);
        assert_eq!(decoded.session_token, credentials.session_token);
    }

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
            access_key_id: "AKIDEXAMPLE".to_string().into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY"
                .to_string()
                .into(),
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
            access_key_id: "AKIDEXAMPLE".to_string().into(),
            secret_access_key: "secret".to_string().into(),
            region: "us-east-1".to_string(),
            service: "ce".to_string(),
            session_token: Some("FQoGZ...short".to_string().into()),
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
        assert_eq!(token_header.value.as_str(), "FQoGZ...short");
    }
}
