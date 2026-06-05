mod codec;
mod context;
mod envelope;
mod error;
mod fingerprint;

use std::fmt;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

pub use codec::{decode_b64u, decode_b64u_array, decode_b64u_capped, encode_b64u};
pub use context::RciContext;
pub use envelope::{CiphertextEnvelope, envelope_from_encoded_parts};
pub use error::{RciCryptoError, Result};
pub use fingerprint::{rci_pubkey_fingerprint, rci_pubkey_fingerprint_b64u};

pub const VERSION_V1: &str = "v1";
pub const MAX_CIPHERTEXT_SIZE: usize = 16 * 1024;

pub struct NodeKeypair {
    private_key: Zeroizing<[u8; 32]>,
    public_key: [u8; 32],
}

impl NodeKeypair {
    pub fn private_key(&self) -> &Zeroizing<[u8; 32]> {
        &self.private_key
    }

    pub fn public_key(&self) -> &[u8; 32] {
        &self.public_key
    }

    pub fn public_key_b64u(&self) -> String {
        encode_b64u(&self.public_key)
    }
}

impl fmt::Debug for NodeKeypair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeKeypair")
            .field("private_key", &"[REDACTED]")
            .field("public_key", &"[REDACTED]")
            .finish()
    }
}

pub fn generate_node_keypair() -> NodeKeypair {
    let secret = StaticSecret::random_from_rng(OsRng);
    let public = PublicKey::from(&secret);
    NodeKeypair {
        private_key: Zeroizing::new(secret.to_bytes()),
        public_key: *public.as_bytes(),
    }
}

pub fn encrypt(
    plaintext: &[u8],
    recipient_pubkey: [u8; 32],
    context: &RciContext,
) -> Result<CiphertextEnvelope> {
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut nonce);
    let admin_secret = StaticSecret::random_from_rng(OsRng);
    encrypt_with_secret_and_nonce(plaintext, recipient_pubkey, context, admin_secret, nonce)
}

fn encrypt_with_secret_and_nonce(
    plaintext: &[u8],
    recipient_pubkey: [u8; 32],
    context: &RciContext,
    admin_secret: StaticSecret,
    nonce: [u8; 24],
) -> Result<CiphertextEnvelope> {
    ensure_v1(&context.version)?;
    let admin_public = PublicKey::from(&admin_secret);
    let shared = admin_secret.diffie_hellman(&PublicKey::from(recipient_pubkey));
    let key = derive_key(shared.as_bytes(), &context.kdf_info_bytes()?)?;
    let aad = context.aad_bytes()?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key.as_slice()));
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| RciCryptoError::Crypto)?;
    if ciphertext.len() > MAX_CIPHERTEXT_SIZE {
        return Err(RciCryptoError::CiphertextTooLarge {
            actual: ciphertext.len(),
            max: MAX_CIPHERTEXT_SIZE,
        });
    }
    Ok(CiphertextEnvelope::new(
        VERSION_V1,
        *admin_public.as_bytes(),
        nonce,
        ciphertext,
    ))
}

fn derive_key(shared_secret: &[u8; 32], info: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
    let hkdf = Hkdf::<Sha256>::new(None, shared_secret);
    let mut key = Zeroizing::new([0u8; 32]);
    hkdf.expand(info, &mut *key)
        .map_err(|_| RciCryptoError::Crypto)?;
    Ok(key)
}

fn ensure_v1(version: &str) -> Result<()> {
    if version == VERSION_V1 {
        Ok(())
    } else {
        Err(RciCryptoError::UnsupportedVersion(version.to_string()))
    }
}

#[cfg(feature = "decrypt")]
pub fn encode_private_key_b64u(private_key: &Zeroizing<[u8; 32]>) -> Zeroizing<String> {
    Zeroizing::new(encode_b64u(private_key.as_slice()))
}

#[cfg(feature = "decrypt")]
pub fn decode_private_key_b64u(encoded: &str) -> Result<Zeroizing<[u8; 32]>> {
    decode_b64u_array("private_key", encoded).map(Zeroizing::new)
}

#[cfg(feature = "decrypt")]
pub fn decrypt(
    envelope: &CiphertextEnvelope,
    private_key: &Zeroizing<[u8; 32]>,
    context: &RciContext,
) -> Result<Zeroizing<Vec<u8>>> {
    ensure_v1(&envelope.version)?;
    ensure_v1(&context.version)?;
    if envelope.ciphertext.len() > MAX_CIPHERTEXT_SIZE {
        return Err(RciCryptoError::CiphertextTooLarge {
            actual: envelope.ciphertext.len(),
            max: MAX_CIPHERTEXT_SIZE,
        });
    }
    let node_secret = StaticSecret::from(**private_key);
    let shared = node_secret.diffie_hellman(&PublicKey::from(envelope.admin_pubkey));
    let key = derive_key(shared.as_bytes(), &context.kdf_info_bytes()?)?;
    let aad = context.aad_bytes()?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key.as_slice()));
    let plaintext = cipher
        .decrypt(
            XNonce::from_slice(&envelope.nonce),
            Payload {
                msg: envelope.ciphertext.as_slice(),
                aad: &aad,
            },
        )
        .map_err(|_| RciCryptoError::Crypto)?;
    Ok(Zeroizing::new(plaintext))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    fn context() -> RciContext {
        RciContext::new(
            "node-123",
            "pending-456",
            "openai",
            "header",
            "Authorization",
            Some("https://api.openai.com/v1".to_string()),
            VERSION_V1,
        )
    }

    #[cfg(feature = "decrypt")]
    #[test]
    fn encrypt_decrypt_roundtrip() {
        let keypair = generate_node_keypair();
        let ctx = context();
        let envelope = encrypt(b"Bearer sk-test", *keypair.public_key(), &ctx).unwrap();
        let plaintext = decrypt(&envelope, keypair.private_key(), &ctx).unwrap();
        assert_eq!(plaintext.as_slice(), b"Bearer sk-test");
    }

    #[cfg(feature = "decrypt")]
    #[test]
    fn wrong_aad_rejected() {
        let keypair = generate_node_keypair();
        let ctx = context();
        let envelope = encrypt(b"Bearer sk-test", *keypair.public_key(), &ctx).unwrap();
        let mut wrong = ctx.clone();
        wrong.field_name = "X-Api-Key".to_string();
        assert!(decrypt(&envelope, keypair.private_key(), &wrong).is_err());
    }

    #[cfg(feature = "decrypt")]
    #[test]
    fn wrong_recipient_rejected() {
        let keypair = generate_node_keypair();
        let other = generate_node_keypair();
        let ctx = context();
        let envelope = encrypt(b"Bearer sk-test", *keypair.public_key(), &ctx).unwrap();
        assert!(decrypt(&envelope, other.private_key(), &ctx).is_err());
    }

    #[cfg(feature = "decrypt")]
    #[test]
    fn ciphertext_is_authenticated() {
        let keypair = generate_node_keypair();
        let ctx = context();
        let mut envelope = encrypt(b"Bearer sk-test", *keypair.public_key(), &ctx).unwrap();
        envelope.ciphertext[0] ^= 0x01;
        assert!(decrypt(&envelope, keypair.private_key(), &ctx).is_err());
    }

    #[test]
    fn nonce_is_random_per_call() {
        let keypair = generate_node_keypair();
        let ctx = context();
        let a = encrypt(b"same plaintext", *keypair.public_key(), &ctx).unwrap();
        let b = encrypt(b"same plaintext", *keypair.public_key(), &ctx).unwrap();
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn base64url_no_pad_rejects_padding_and_wrong_lengths() {
        assert!(decode_b64u("test", "abcd=").is_err());
        let short = encode_b64u(&[1, 2, 3]);
        assert!(decode_b64u_array::<32>("admin_pubkey", &short).is_err());
    }

    #[test]
    fn encoded_envelope_rejects_oversized_ciphertext() {
        let oversized = vec![0_u8; MAX_CIPHERTEXT_SIZE + 1];
        let error = envelope_from_encoded_parts(
            VERSION_V1,
            &encode_b64u(&[1; 32]),
            &encode_b64u(&[2; 24]),
            &encode_b64u(&oversized),
        )
        .expect_err("oversized encoded envelope should be rejected");

        assert!(matches!(
            error,
            RciCryptoError::CiphertextTooLarge {
                actual,
                max: MAX_CIPHERTEXT_SIZE
            } if actual == MAX_CIPHERTEXT_SIZE + 1
        ));
    }

    #[test]
    fn serde_envelope_rejects_oversized_ciphertext() {
        let value = serde_json::json!({
            "version": VERSION_V1,
            "admin_pubkey": encode_b64u(&[1; 32]),
            "nonce": encode_b64u(&[2; 24]),
            "ciphertext": encode_b64u(&vec![0_u8; MAX_CIPHERTEXT_SIZE + 1]),
        });

        let error = serde_json::from_value::<CiphertextEnvelope>(value)
            .expect_err("oversized serde envelope should be rejected")
            .to_string();

        assert!(error.contains("ciphertext exceeds maximum size"));
        assert!(error.contains(&(MAX_CIPHERTEXT_SIZE + 1).to_string()));
    }

    #[cfg(feature = "decrypt")]
    #[test]
    fn decrypt_rejects_oversized_ciphertext_before_crypto() {
        let envelope = CiphertextEnvelope::new(
            VERSION_V1,
            [1; 32],
            [2; 24],
            vec![0_u8; MAX_CIPHERTEXT_SIZE + 1],
        );
        let private_key = Zeroizing::new([3_u8; 32]);
        let error =
            decrypt(&envelope, &private_key, &context()).expect_err("oversized decrypt input");

        assert!(matches!(
            error,
            RciCryptoError::CiphertextTooLarge {
                actual,
                max: MAX_CIPHERTEXT_SIZE
            } if actual == MAX_CIPHERTEXT_SIZE + 1
        ));
    }

    #[derive(Deserialize)]
    struct Fixture {
        node_private_key: String,
        node_public_key: String,
        admin_private_key: String,
        nonce: String,
        plaintext: String,
        context: FixtureContext,
        envelope: CiphertextEnvelope,
    }

    #[derive(Deserialize)]
    struct FixtureContext {
        node_id: String,
        pending_credential_id: String,
        service_slug: String,
        injection_method: String,
        field_name: String,
        target_url: Option<String>,
        version: String,
    }

    fn fixture_context(ctx: FixtureContext) -> RciContext {
        RciContext::new(
            ctx.node_id,
            ctx.pending_credential_id,
            ctx.service_slug,
            ctx.injection_method,
            ctx.field_name,
            ctx.target_url,
            ctx.version,
        )
    }

    #[cfg(feature = "decrypt")]
    #[test]
    fn fixed_nonce_fixture_matches_expected_bytes() {
        let fixture: Fixture =
            serde_json::from_str(include_str!("../../tests/fixtures/rci/v1_envelope.json"))
                .unwrap();
        let node_private = decode_private_key_b64u(&fixture.node_private_key).unwrap();
        let node_public = decode_b64u_array::<32>("node_public_key", &fixture.node_public_key)
            .expect("fixture node pubkey");
        let admin_private = StaticSecret::from(
            decode_b64u_array::<32>("admin_private_key", &fixture.admin_private_key).unwrap(),
        );
        let nonce = decode_b64u_array::<24>("nonce", &fixture.nonce).unwrap();
        let ctx = fixture_context(fixture.context);
        let generated = encrypt_with_secret_and_nonce(
            fixture.plaintext.as_bytes(),
            node_public,
            &ctx,
            admin_private,
            nonce,
        )
        .unwrap();

        assert_eq!(generated, fixture.envelope);
        let decrypted = decrypt(&fixture.envelope, &node_private, &ctx).unwrap();
        assert_eq!(decrypted.as_slice(), fixture.plaintext.as_bytes());
    }

    #[test]
    fn rci_context_vectors_are_stable() {
        let ctx = context();
        assert_eq!(
            encode_b64u(&ctx.kdf_info_bytes().unwrap()),
            "bnl4aWQ6cmNpOnYxOmtkZgAACG5vZGUtMTIzAAtwZW5kaW5nLTQ1NgAGb3BlbmFpAAJ2MQ"
        );
        assert_eq!(
            encode_b64u(&ctx.aad_bytes().unwrap()),
            "bnl4aWQ6cmNpOnYxOmFhZAAACG5vZGUtMTIzAAtwZW5kaW5nLTQ1NgAGb3BlbmFpAAZoZWFkZXIADUF1dGhvcml6YXRpb24AGWh0dHBzOi8vYXBpLm9wZW5haS5jb20vdjEAAnYx"
        );
    }

    #[test]
    fn debug_redacts_key_material_and_ciphertext() {
        let envelope = CiphertextEnvelope::new(VERSION_V1, [1; 32], [2; 24], vec![3, 4, 5]);
        let debug = format!("{envelope:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("[1, 1"));
        assert!(!debug.contains("[2, 2"));
        assert!(!debug.contains("[3, 4, 5]"));

        let keypair = generate_node_keypair();
        let debug = format!("{keypair:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains(&keypair.public_key_b64u()));
    }
}
