//! DPoP (RFC 9449) proof validation for sender-constrained access tokens.
//!
//! Scoped to ES256 in v1; broader algorithms can be added later. Replay
//! protection is an atomic MongoDB claim shared by every replica.

use base64::Engine as _;
use chrono::Utc;
use jsonwebtoken::jwk::{AlgorithmParameters, EllipticCurve, Jwk};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::Deserialize;
use url::Url;

use crate::errors::{AppError, AppResult};
use crate::services::dpop_jti_cache;

const DPOP_ACCEPTED_TYP: &str = "dpop+jwt";
const DPOP_IAT_WINDOW_SECS: i64 = 300;

#[derive(Debug, Deserialize)]
struct DpopHeader {
    typ: String,
    alg: String,
    jwk: Jwk,
}

#[derive(Debug, Deserialize)]
struct DpopClaims {
    htm: String,
    htu: String,
    iat: i64,
    jti: String,
}

/// Build the canonical htu value for a request path under the configured base URL.
pub fn htu_from_base_and_path(base_url: &str, path: &str) -> AppResult<String> {
    let mut base = Url::parse(base_url.trim_end_matches('/'))
        .map_err(|_| AppError::Unauthorized("invalid DPoP htu".to_string()))?;
    base.set_path(path);
    base.set_query(None);
    base.set_fragment(None);
    canonicalize_htu(base.as_str())
}

/// Validate a DPoP proof JWT against the request method and URI.
/// Returns the JWK thumbprint (RFC 7638) on success.
pub async fn validate_proof(
    dpop_header: &str,
    expected_method: &str,
    expected_htu: &str,
    db: &mongodb::Database,
) -> AppResult<String> {
    let validated = validate_proof_claims(dpop_header, expected_method, expected_htu)?;
    if !dpop_jti_cache::claim_jti(db, &validated.jti).await? {
        return Err(AppError::Unauthorized("DPoP replay detected".to_string()));
    }
    Ok(validated.thumbprint)
}

struct ValidatedDpopProof {
    thumbprint: String,
    jti: String,
}

fn validate_proof_claims(
    dpop_header: &str,
    expected_method: &str,
    expected_htu: &str,
) -> AppResult<ValidatedDpopProof> {
    let header_segment = dpop_header
        .split('.')
        .next()
        .ok_or_else(|| AppError::Unauthorized("invalid DPoP proof".to_string()))?;
    let header_json = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(header_segment)
        .map_err(|_| AppError::Unauthorized("invalid DPoP proof header".to_string()))?;
    let header: DpopHeader = serde_json::from_slice(&header_json)
        .map_err(|_| AppError::Unauthorized("invalid DPoP proof header".to_string()))?;

    if header.typ != DPOP_ACCEPTED_TYP {
        return Err(AppError::Unauthorized("unsupported DPoP typ".to_string()));
    }
    if header.alg != "ES256" {
        return Err(AppError::Unauthorized("unsupported DPoP alg".to_string()));
    }

    let key = DecodingKey::from_jwk(&header.jwk)
        .map_err(|_| AppError::Unauthorized("invalid DPoP key".to_string()))?;

    let mut validation = Validation::new(Algorithm::ES256);
    validation.required_spec_claims.clear();
    validation.validate_exp = false;
    validation.validate_nbf = false;
    validation.validate_aud = false;
    let data = decode::<DpopClaims>(dpop_header, &key, &validation)
        .map_err(|_| AppError::Unauthorized("invalid DPoP signature".to_string()))?;

    let claims = data.claims;
    if claims.htm != expected_method {
        return Err(AppError::Unauthorized("DPoP htm mismatch".to_string()));
    }

    let expected_htu = canonicalize_htu(expected_htu)?;
    let proof_htu = canonicalize_htu(&claims.htu)?;
    if proof_htu != expected_htu {
        return Err(AppError::Unauthorized("DPoP htu mismatch".to_string()));
    }

    let now = Utc::now().timestamp();
    if (now - claims.iat).abs() > DPOP_IAT_WINDOW_SECS {
        return Err(AppError::Unauthorized(
            "DPoP iat outside window".to_string(),
        ));
    }
    Ok(ValidatedDpopProof {
        thumbprint: jwk_thumbprint(&header.jwk),
        jti: claims.jti,
    })
}

/// Compute SHA-256 thumbprint of a JWK per RFC 7638.
pub fn jwk_thumbprint(jwk: &Jwk) -> String {
    use sha2::{Digest, Sha256};

    let canonical = match &jwk.algorithm {
        AlgorithmParameters::EllipticCurve(params) => format!(
            r#"{{"crv":"{}","kty":"EC","x":"{}","y":"{}"}}"#,
            curve_name(&params.curve),
            params.x,
            params.y
        ),
        AlgorithmParameters::RSA(params) => {
            format!(r#"{{"e":"{}","kty":"RSA","n":"{}"}}"#, params.e, params.n)
        }
        _ => return String::new(),
    };
    let digest = Sha256::digest(canonical.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn curve_name(curve: &EllipticCurve) -> &'static str {
    match curve {
        EllipticCurve::P256 => "P-256",
        EllipticCurve::P384 => "P-384",
        EllipticCurve::P521 => "P-521",
        EllipticCurve::Ed25519 => "Ed25519",
    }
}

fn canonicalize_htu(uri: &str) -> AppResult<String> {
    let mut parsed =
        Url::parse(uri).map_err(|_| AppError::Unauthorized("invalid DPoP htu".to_string()))?;
    parsed.set_query(None);
    parsed.set_fragment(None);

    let scheme = parsed.scheme().to_ascii_lowercase();
    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::Unauthorized("invalid DPoP htu".to_string()))?
        .to_ascii_lowercase();
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host
    };
    let port = parsed
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    Ok(format!("{scheme}://{host}{port}{}", parsed.path()))
}

#[cfg(test)]
pub(crate) fn test_dpop_keypair() -> (jsonwebtoken::EncodingKey, Jwk) {
    use jsonwebtoken::EncodingKey;
    use p256::ecdsa::SigningKey;
    use p256::pkcs8::{EncodePrivateKey, LineEnding};

    let signing_key = SigningKey::random(&mut rand::rngs::OsRng);
    let private_pem = signing_key
        .to_pkcs8_pem(LineEnding::LF)
        .expect("encode P-256 private key");
    let encoding_key = EncodingKey::from_ec_pem(private_pem.as_bytes()).expect("EC encoding key");
    let jwk = Jwk::from_encoding_key(&encoding_key, Algorithm::ES256).expect("derive public JWK");
    (encoding_key, jwk)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::jwk::{
        AlgorithmParameters, CommonParameters, OctetKeyPairParameters, OctetKeyPairType,
        RSAKeyParameters, RSAKeyType,
    };
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde::Serialize;
    use uuid::Uuid;

    #[derive(Serialize)]
    struct TestDpopClaims {
        htm: String,
        htu: String,
        iat: i64,
        jti: String,
    }

    pub(crate) fn sign_test_proof(
        encoding_key: &EncodingKey,
        jwk: &Jwk,
        htm: &str,
        htu: &str,
        iat: i64,
        jti: &str,
    ) -> String {
        let mut header = Header::new(Algorithm::ES256);
        header.typ = Some(DPOP_ACCEPTED_TYP.to_string());
        header.jwk = Some(jwk.clone());
        encode(
            &header,
            &TestDpopClaims {
                htm: htm.to_string(),
                htu: htu.to_string(),
                iat,
                jti: jti.to_string(),
            },
            encoding_key,
        )
        .expect("sign DPoP proof")
    }

    /// RFC 7638 Appendix A.1 test vector for an RSA JWK thumbprint.
    #[test]
    fn rfc7638_rsa_thumbprint_test_vector() {
        let jwk = Jwk {
            common: CommonParameters::default(),
            algorithm: AlgorithmParameters::RSA(RSAKeyParameters {
                key_type: RSAKeyType::RSA,
                n: "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw".to_string(),
                e: "AQAB".to_string(),
            }),
        };
        assert_eq!(
            jwk_thumbprint(&jwk),
            "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs"
        );
    }

    #[tokio::test]
    async fn validate_proof_round_trip() {
        let Some(db) = crate::test_utils::connect_test_database("dpop_round_trip").await else {
            return;
        };
        let (encoding_key, jwk) = test_dpop_keypair();
        let htu = "https://auth.example.com/oauth/token";
        let proof = sign_test_proof(
            &encoding_key,
            &jwk,
            "POST",
            htu,
            Utc::now().timestamp(),
            &Uuid::new_v4().to_string(),
        );

        let thumbprint = validate_proof(&proof, "POST", htu, &db)
            .await
            .expect("valid DPoP");
        assert_eq!(thumbprint, jwk_thumbprint(&jwk));
    }

    #[tokio::test]
    async fn validate_proof_rejects_replay_across_callers() {
        let Some(db) = crate::test_utils::connect_test_database("dpop_replay").await else {
            return;
        };
        let (encoding_key, jwk) = test_dpop_keypair();
        let htu = "https://auth.example.com/oauth/token";
        let proof = sign_test_proof(
            &encoding_key,
            &jwk,
            "POST",
            htu,
            Utc::now().timestamp(),
            "jti-replay",
        );

        validate_proof(&proof, "POST", htu, &db)
            .await
            .expect("first proof");
        assert!(validate_proof(&proof, "POST", htu, &db).await.is_err());
    }

    #[test]
    fn validate_proof_rejects_stale_iat() {
        let (encoding_key, jwk) = test_dpop_keypair();
        let htu = "https://auth.example.com/oauth/token";
        let proof = sign_test_proof(
            &encoding_key,
            &jwk,
            "POST",
            htu,
            Utc::now().timestamp() - DPOP_IAT_WINDOW_SECS - 1,
            &Uuid::new_v4().to_string(),
        );

        assert!(validate_proof_claims(&proof, "POST", htu).is_err());
    }

    #[test]
    fn validate_proof_rejects_htm_mismatch() {
        let (encoding_key, jwk) = test_dpop_keypair();
        let htu = "https://auth.example.com/oauth/token";
        let proof = sign_test_proof(
            &encoding_key,
            &jwk,
            "GET",
            htu,
            Utc::now().timestamp(),
            &Uuid::new_v4().to_string(),
        );

        assert!(validate_proof_claims(&proof, "POST", htu).is_err());
    }

    #[test]
    fn canonicalize_htu_strips_query_and_fragment() {
        let result = canonicalize_htu("https://example.com/path?q=1#frag").unwrap();
        assert_eq!(result, "https://example.com/path");
    }

    #[test]
    fn canonicalize_htu_lowercases_scheme_and_host() {
        let result = canonicalize_htu("HTTPS://EXAMPLE.COM/Path").unwrap();
        assert_eq!(result, "https://example.com/Path");
    }

    #[test]
    fn canonicalize_htu_rejects_invalid_uri() {
        assert!(canonicalize_htu("not a url").is_err());
    }

    #[test]
    fn htu_from_base_and_path_constructs_correctly() {
        let htu = htu_from_base_and_path("https://auth.example.com/", "/oauth/token").unwrap();
        assert_eq!(htu, "https://auth.example.com/oauth/token");
    }

    #[test]
    fn jwk_thumbprint_ec_key() {
        let (_, jwk) = test_dpop_keypair();
        let t = jwk_thumbprint(&jwk);
        assert!(!t.is_empty());
        // base64url-no-pad encoded SHA-256 should be 43 chars
        assert_eq!(t.len(), 43);
    }

    #[test]
    fn jwk_thumbprint_unknown_kty_returns_empty() {
        let jwk = Jwk {
            common: CommonParameters::default(),
            algorithm: AlgorithmParameters::OctetKeyPair(OctetKeyPairParameters {
                key_type: OctetKeyPairType::OctetKeyPair,
                curve: EllipticCurve::Ed25519,
                x: "test_x".to_string(),
            }),
        };
        assert_eq!(jwk_thumbprint(&jwk), "");
    }

    #[test]
    fn validate_proof_rejects_htu_mismatch() {
        let (encoding_key, jwk) = test_dpop_keypair();
        let proof = sign_test_proof(
            &encoding_key,
            &jwk,
            "POST",
            "https://wrong.com/token",
            Utc::now().timestamp(),
            &Uuid::new_v4().to_string(),
        );
        assert!(validate_proof_claims(&proof, "POST", "https://right.com/token").is_err());
    }
}
