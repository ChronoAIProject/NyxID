//! Concrete verification of provider workload tickets. Configuring a key
//! trusts its issuer; it does not prove workload delivery, runtime isolation or
//! physical writer fencing. Those remain independent deployment obligations.

use std::{path::Path, sync::Arc};

use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use nyxid_machine::managed::Binding;
use rsa::{pkcs1::DecodeRsaPublicKey, pkcs8::DecodePublicKey, traits::PublicKeyParts};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::remote_machine_bootstrap::{
    BootstrapTicketVerifier, UnavailableBootstrapTicketVerifier, VerifiedBootstrapTicket,
};
use crate::errors::{AppError, AppResult};

pub const AUDIENCE: &str = "nyxid-remote-machine-v1";
pub const PURPOSE: &str = "bootstrap";

pub struct SignedWorkloadTicketVerifier {
    key: DecodingKey,
    issuer: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct Claims {
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    purpose: String,
    binding: Binding,
    /// SHA-256 of the recipient's canonical RSA-3072 SPKI DER public key. The
    /// trusted platform signs this only after authenticating the target Pod
    /// and its protected preparation request; a bearer ticket cannot be
    /// redirected to another recipient on its first use.
    recipient_key_digest: String,
}

fn invalid_ticket() -> AppError {
    AppError::Unauthorized("managed workload ticket is invalid or expired".into())
}

impl SignedWorkloadTicketVerifier {
    pub fn from_public_key_file(path: &Path, issuer: &str) -> AppResult<Self> {
        validate_provider_issuer(issuer)?;
        Ok(Self {
            key: load_provider_public_key(path)?,
            issuer: issuer.into(),
        })
    }
}

pub(crate) fn validate_provider_issuer(issuer: &str) -> AppResult<()> {
    if issuer.trim().is_empty() || issuer.len() > 1024 || issuer.chars().any(char::is_control) {
        return Err(AppError::ValidationError(
            "invalid managed provider configuration".into(),
        ));
    }
    Ok(())
}

/// Both workload tickets and physical-evidence transport use the dedicated
/// provider key contract; malformed/weak configuration leaves admission closed.
pub(crate) fn load_provider_public_key(path: &Path) -> AppResult<DecodingKey> {
    let metadata = std::fs::metadata(path)
        .map_err(|_| AppError::ValidationError("invalid managed provider configuration".into()))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 64 * 1024 {
        return Err(AppError::ValidationError(
            "invalid managed provider configuration".into(),
        ));
    }
    let pem = std::fs::read(path)
        .map_err(|_| AppError::ValidationError("invalid managed provider configuration".into()))?;
    let text = std::str::from_utf8(&pem)
        .map_err(|_| AppError::ValidationError("invalid managed provider configuration".into()))?;
    let rsa = rsa::RsaPublicKey::from_public_key_pem(text)
        .or_else(|_| rsa::RsaPublicKey::from_pkcs1_pem(text))
        .map_err(|_| AppError::ValidationError("invalid managed provider configuration".into()))?;
    if rsa.n().bits() < 2048 {
        return Err(AppError::ValidationError(
            "invalid managed provider configuration".into(),
        ));
    }
    DecodingKey::from_rsa_pem(&pem)
        .map_err(|_| AppError::ValidationError("invalid managed provider configuration".into()))
}

#[async_trait::async_trait]
impl BootstrapTicketVerifier for SignedWorkloadTicketVerifier {
    async fn verify(
        &self,
        ticket: &Value,
        binding: &Binding,
    ) -> AppResult<VerifiedBootstrapTicket> {
        let token = ticket
            .get("token")
            .and_then(Value::as_str)
            .ok_or_else(invalid_ticket)?;
        if token.is_empty() || token.len() > 48 * 1024 {
            return Err(invalid_ticket());
        }
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&self.issuer]);
        validation.set_audience(&[AUDIENCE]);
        validation.set_required_spec_claims(&["iss", "aud", "iat", "exp"]);
        validation.leeway = 0;
        let claims = jsonwebtoken::decode::<Claims>(token, &self.key, &validation)
            .map_err(|_| invalid_ticket())?
            .claims;
        let now = Utc::now().timestamp();
        let lifetime = claims
            .exp
            .checked_sub(claims.iat)
            .filter(|seconds| *seconds > 0 && *seconds <= 900);
        if claims.purpose != PURPOSE
            || claims.binding != *binding
            || claims.iat > now
            || claims.exp <= now
            || lifetime.is_none()
        {
            return Err(invalid_ticket());
        }
        claims.binding.validate().map_err(|_| invalid_ticket())?;
        let recipient_key_digest: [u8; 32] = hex::decode(&claims.recipient_key_digest)
            .map_err(|_| invalid_ticket())?
            .try_into()
            .map_err(|_| invalid_ticket())?;
        if hex::encode(recipient_key_digest) != claims.recipient_key_digest {
            return Err(invalid_ticket());
        }
        let expires_at =
            chrono::DateTime::from_timestamp(claims.exp, 0).ok_or_else(invalid_ticket)?;
        Ok(VerifiedBootstrapTicket::from_provider_verified_claims(
            claims.binding,
            recipient_key_digest,
            expires_at,
        ))
    }
}

/// Optional deployment configuration. Missing or malformed configuration never
/// makes the main NyxID server unavailable and never admits a managed workload.
/// A provider key is dedicated to this protocol, not the NyxID user JWT key.
pub fn configured_verifier() -> Arc<dyn BootstrapTicketVerifier> {
    // The selected Kubernetes platform already issues Pod-bound tokens. Use
    // TokenReview rather than requiring a separately deployed ticket issuer.
    if std::env::var_os("NYXID_REMOTE_MACHINE_KUBE_API").is_some() {
        return match super::remote_machine_kubernetes_identity::KubernetesWorkloadVerifier::from_environment() {
            Ok(verifier) => Arc::new(verifier),
            Err(_) => {
                tracing::error!("Managed Kubernetes identity configuration invalid; admission remains closed");
                Arc::new(UnavailableBootstrapTicketVerifier)
            }
        };
    }
    let path = std::env::var("NYXID_REMOTE_MACHINE_PROVIDER_PUBLIC_KEY_FILE").ok();
    let issuer = std::env::var("NYXID_REMOTE_MACHINE_PROVIDER_ISSUER").ok();
    match (path, issuer) {
        (Some(path), Some(issuer)) => {
            match SignedWorkloadTicketVerifier::from_public_key_file(Path::new(&path), &issuer) {
                Ok(verifier) => Arc::new(verifier),
                Err(_) => {
                    tracing::error!(
                        "Managed workload ticket verification configuration is invalid; admission remains closed"
                    );
                    Arc::new(UnavailableBootstrapTicketVerifier)
                }
            }
        }
        (None, None) => Arc::new(UnavailableBootstrapTicketVerifier),
        _ => {
            tracing::error!(
                "Managed workload ticket verification configuration is incomplete; admission remains closed"
            );
            Arc::new(UnavailableBootstrapTicketVerifier)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header};

    fn binding() -> Binding {
        Binding {
            node_id: uuid::Uuid::new_v4().to_string(),
            sandbox_uid: uuid::Uuid::new_v4().to_string(),
            pod_uid: uuid::Uuid::new_v4().to_string(),
            runtime_generation: 1,
            credential_epoch: 1,
            writer_fence: uuid::Uuid::new_v4().to_string(),
            operation_id: uuid::Uuid::new_v4().to_string(),
            delivery_id: uuid::Uuid::new_v4().to_string(),
        }
    }
    fn claims() -> Claims {
        Claims {
            iss: "test-provider".into(),
            aud: AUDIENCE.into(),
            iat: Utc::now().timestamp(),
            exp: Utc::now().timestamp() + 60,
            purpose: PURPOSE.into(),
            binding: binding(),
            recipient_key_digest: hex::encode([7_u8; 32]),
        }
    }
    fn fixture() -> (SignedWorkloadTicketVerifier, EncodingKey) {
        let keys = crate::test_utils::cached_test_jwt_keys();
        (
            SignedWorkloadTicketVerifier {
                key: keys.decoding,
                issuer: "test-provider".into(),
            },
            keys.encoding,
        )
    }
    fn ticket(claims: &Claims, key: &EncodingKey) -> Value {
        serde_json::json!({"token":jsonwebtoken::encode(&Header::new(Algorithm::RS256), claims, key).unwrap()})
    }

    #[tokio::test]
    async fn remote_machine_bootstrap_verifier_accepts_only_exact_signed_short_lived_ticket() {
        let (verifier, key) = fixture();
        let claims = claims();
        assert!(
            verifier
                .verify(&ticket(&claims, &key), &claims.binding)
                .await
                .is_ok()
        );
        for field in [
            "issuer",
            "audience",
            "purpose",
            "expiry",
            "future",
            "lifetime",
            "binding",
            "recipient_digest",
        ] {
            let mut changed = claims.clone();
            match field {
                "issuer" => changed.iss = "other-provider".into(),
                "audience" => changed.aud = "user-access".into(),
                "purpose" => changed.purpose = "runtime".into(),
                "expiry" => changed.exp = Utc::now().timestamp() - 1,
                "future" => changed.iat += 30,
                "lifetime" => changed.exp = changed.iat + 901,
                "binding" => changed.binding.pod_uid = uuid::Uuid::new_v4().to_string(),
                "recipient_digest" => changed.recipient_key_digest = "not-a-key-digest".into(),
                _ => unreachable!(),
            }
            assert!(
                verifier
                    .verify(&ticket(&changed, &key), &claims.binding)
                    .await
                    .is_err(),
                "{field}"
            );
        }
    }

    #[tokio::test]
    async fn remote_machine_bootstrap_verifier_requires_signed_recipient_binding() {
        let (verifier, key) = fixture();
        let claims = claims();
        let mut unbound = serde_json::to_value(&claims).unwrap();
        unbound
            .as_object_mut()
            .unwrap()
            .remove("recipient_key_digest");
        let token = jsonwebtoken::encode(&Header::new(Algorithm::RS256), &unbound, &key).unwrap();
        assert!(
            verifier
                .verify(&serde_json::json!({"token": token}), &claims.binding)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn remote_machine_bootstrap_verifier_rejects_tampering_unsigned_and_wrong_algorithm() {
        let (verifier, key) = fixture();
        let claims = claims();
        for token in ["", "eyJhbGciOiJub25lIn0.e30.", "malformed"] {
            assert!(
                verifier
                    .verify(
                        &serde_json::json!({"token":token,"verified":true}),
                        &claims.binding
                    )
                    .await
                    .is_err()
            );
        }
        let token = jsonwebtoken::encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(b"not-a-provider-key"),
        )
        .unwrap();
        assert!(
            verifier
                .verify(&serde_json::json!({"token":token}), &claims.binding)
                .await
                .is_err()
        );
        let signed = ticket(&claims, &key);
        let token = signed["token"].as_str().unwrap();
        let (body, signature) = token.rsplit_once('.').unwrap();
        let mut changed = signature.as_bytes().to_vec();
        changed[0] = if changed[0] == b'A' { b'B' } else { b'A' };
        let tampered = format!("{body}.{}", String::from_utf8(changed).unwrap());
        assert!(
            verifier
                .verify(&serde_json::json!({"token":tampered}), &claims.binding)
                .await
                .is_err()
        );
        assert!(
            verifier
                .verify(&serde_json::json!({"verified":true}), &claims.binding)
                .await
                .is_err()
        );
    }

    #[test]
    fn remote_machine_bootstrap_verifier_rejects_invalid_public_key_configuration() {
        let file = tempfile::NamedTempFile::new().unwrap();
        assert!(SignedWorkloadTicketVerifier::from_public_key_file(file.path(), "issuer").is_err());
        std::fs::write(file.path(), b"not-a-key").unwrap();
        assert!(SignedWorkloadTicketVerifier::from_public_key_file(file.path(), "issuer").is_err());
        assert!(SignedWorkloadTicketVerifier::from_public_key_file(file.path(), "").is_err());
    }
}
