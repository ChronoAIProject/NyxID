//! RFC 8705 certificate-bound access token helpers.
//!
//! NyxID does not terminate mTLS itself. Deployments that want certificate-bound
//! tokens must terminate mTLS at a trusted reverse proxy and forward the client
//! certificate in an admin-configured header. This module parses that forwarded
//! PEM payload and computes the RFC 8705 `x5t#S256` value.

use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::errors::{AppError, AppResult};

const PEM_BEGIN: &str = "-----BEGIN CERTIFICATE-----";
const PEM_END: &str = "-----END CERTIFICATE-----";

/// Parse a URL-decoded or raw PEM client certificate from an HTTP header and
/// return the SHA-256 thumbprint over the DER bytes as base64url-no-pad.
pub fn cert_thumbprint_from_header(header_value: &str) -> AppResult<String> {
    let pem = match urlencoding::decode(header_value) {
        Ok(decoded) if decoded.contains(PEM_BEGIN) => decoded.into_owned(),
        _ => header_value.to_string(),
    };

    let der = pem_to_der(&pem)?;
    let digest = Sha256::digest(&der);
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest))
}

fn pem_to_der(pem: &str) -> AppResult<Vec<u8>> {
    let begin = pem
        .find(PEM_BEGIN)
        .ok_or_else(|| AppError::Unauthorized("invalid client certificate".to_string()))?;
    let body_start = begin + PEM_BEGIN.len();
    let end = pem[body_start..]
        .find(PEM_END)
        .map(|offset| body_start + offset)
        .ok_or_else(|| AppError::Unauthorized("invalid client certificate".to_string()))?;
    let body = &pem[body_start..end];
    let cleaned: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(cleaned)
        .map_err(|_| AppError::Unauthorized("invalid client certificate".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_CERT_PEM: &str = "\
-----BEGIN CERTIFICATE-----
MIIDCzCCAfOgAwIBAgIUCcPX8npxOnTmISZA70uWhCFOybwwDQYJKoZIhvcNAQEL
BQAwFTETMBEGA1UEAwwKbnl4aWQtdGVzdDAeFw0yNjA0MjgxNDE2MThaFw0yNjA0
MjkxNDE2MThaMBUxEzARBgNVBAMMCm55eGlkLXRlc3QwggEiMA0GCSqGSIb3DQEB
AQUAA4IBDwAwggEKAoIBAQC5UP80pjEa2WnNnDrrayvqv1HoLcc+xff23h4CfhxW
ud2JDnDm2CtHeR4YEnWwxjoSmOzqWxWz8p4aiVENWHQAOJ6NHGqzVbSTsMAokwcb
Hu9rhRIwaOhKuNqT6XtvIzcogrEnamJTOZb+BCnSBHtNXjyTJ5foOIm4k7gUnd1D
7+9L6gltRjXEBJezJ/JKW/p9VXxA+3Ib4FMbBoY702fH6+8MC3N2qoXbUbOH2Z7c
c5OXhQCxAJeA6Q5O8ME9AYbSsfMPyemp4qHgj19v3Ld9C8D9TLQtvXcmkYChrwqP
NJqt6I6v9gS5DLxpU+4m7Y5CwgX8tAUhtkqBkUyWHbJtAgMBAAGjUzBRMB0GA1Ud
DgQWBBSYuylU4OSDstJYzIITptD1Luir2TAfBgNVHSMEGDAWgBSYuylU4OSDstJY
zIITptD1Luir2TAPBgNVHRMBAf8EBTADAQH/MA0GCSqGSIb3DQEBCwUAA4IBAQAv
FeZrwCRCPNvtis/NfLhVKp+2WWsnTXdVzcnSEucBFrXcW+upEb1/YSmcJo8tcRXj
RBUCOlfU0SI77I+c4Re2nPr4vDqKeJG/+pdmIjSTxLXtItbSeTE8Lk04hfQfGAqy
X8E8PtlOLZqYNuDH9/8/nowS992JlzTOjEkgpjv2M7QEPFewr4Ov6E2Ue/7a+t6q
101SPegrUfjFo1UFR52M1Qicdk0Y+DwVvigG3IXcbBhl6udNQ7hjKvZCyckE31Hr
VwfvwqZigb9DbGUq+97XHd5eB0Z5G1duKYi4NcVf388whOW0GvgN2oJeev7duFPY
p2xMZskTPAC/OuqUjALe
-----END CERTIFICATE-----
";
    const TEST_CERT_THUMBPRINT: &str = "1q5QYYtDLqlZwvuYfsu07OIGzNzCJgkpf0aCKmOT6Yw";

    #[test]
    fn cert_thumbprint_round_trip() {
        let thumbprint = cert_thumbprint_from_header(TEST_CERT_PEM).expect("thumbprint");
        assert_eq!(thumbprint, TEST_CERT_THUMBPRINT);
    }

    #[test]
    fn cert_thumbprint_handles_url_encoding() {
        let encoded = urlencoding::encode(TEST_CERT_PEM);
        let thumbprint = cert_thumbprint_from_header(&encoded).expect("thumbprint");
        assert_eq!(thumbprint, TEST_CERT_THUMBPRINT);
    }

    #[test]
    fn cert_thumbprint_rejects_garbage() {
        let result = cert_thumbprint_from_header("not a cert");
        assert!(result.is_err());
    }
}
