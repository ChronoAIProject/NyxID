//! Recipient-bound delivery for managed machine bootstrap credentials.
//!
//! Version 1 uses RSA-3072 OAEP (SHA-256, MGF1 SHA-256) to wrap a random
//! AES-256-GCM key. Public keys are canonical unpadded base64url SPKI DER;
//! persisted private keys are canonical unpadded base64url PKCS#8 DER. There
//! is no fallback to any previous bootstrap key or envelope representation.
//!
//! OAEP's label is [`DOMAIN`]. GCM authenticates `DOMAIN || 0x00 || version`
//! (the version is a four-byte big-endian integer) followed by the complete
//! ordered JSON returned by [`managed::Binding::context_field`]. Bootstrap
//! admission, ticket lifetimes, retries, and credential activation belong to
//! callers; possession of an envelope does not authorize an execution.

use crate::managed;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce, Tag, aead::AeadInPlace};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};
use rsa::{
    Oaep, RsaPrivateKey, RsaPublicKey,
    pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePublicKey, der::Encode},
    traits::{PrivateKeyParts, PublicKeyParts},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use zeroize::Zeroizing;

pub const DOMAIN: &str = "nyxid-machine-bootstrap-v1";
pub const VERSION: u32 = 1;
/// Keeps the base64 envelope inside the existing 64 KiB bootstrap HTTP cap.
pub const MAX_PLAINTEXT_BYTES: usize = 16 * 1024;
const RSA_BITS: usize = 3072;
const RSA_BYTES: usize = RSA_BITS / 8;
const PUBLIC_DER_BYTES: usize = 422;
const MAX_PRIVATE_DER_BYTES: usize = 2048;
const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;

/// The RSA implementation zeroizes its private components and precomputations
/// on drop. Private DER, unwrapped AES keys, and plaintext use Zeroizing buffers.
pub struct RecipientKey {
    key: RsaPrivateKey,
}

impl fmt::Debug for RecipientKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RecipientKey { [REDACTED] }")
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub version: u32,
    pub wrapped_key: String,
    pub nonce: String,
    /// Unpadded base64url ciphertext followed by the 16-byte GCM tag.
    pub ciphertext: String,
}

impl fmt::Debug for Envelope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Envelope { [REDACTED] }")
    }
}

/// Deliberately contains no key identifiers, inputs, or upstream crypto errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidKey,
    InvalidBinding,
    InvalidEnvelope,
    TooLarge,
    Cryptography,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidKey => "invalid managed bootstrap recipient key",
            Self::InvalidBinding => "invalid managed bootstrap binding",
            Self::InvalidEnvelope => "invalid managed bootstrap envelope",
            Self::TooLarge => "managed bootstrap size limit exceeded",
            Self::Cryptography => "managed bootstrap cryptography failed",
        })
    }
}

impl std::error::Error for Error {}

/// RSA key generation is CPU-intensive; async callers must use spawn_blocking.
pub fn generate_recipient() -> Result<RecipientKey, Error> {
    let key = RsaPrivateKey::new(&mut OsRng, RSA_BITS).map_err(|_| Error::Cryptography)?;
    Ok(RecipientKey { key })
}

impl RecipientKey {
    pub fn public_key_b64u(&self) -> Result<String, Error> {
        let der = RsaPublicKey::from(&self.key)
            .to_public_key_der()
            .map_err(|_| Error::InvalidKey)?;
        Ok(URL_SAFE_NO_PAD.encode(der.as_bytes()))
    }
}

pub fn encode_private_key(recipient: &RecipientKey) -> Result<Zeroizing<String>, Error> {
    let der = private_der(&recipient.key)?;
    Ok(Zeroizing::new(URL_SAFE_NO_PAD.encode(&*der)))
}

pub fn decode_private_key(encoded: &str) -> Result<RecipientKey, Error> {
    let der = decode_b64(encoded, 1, MAX_PRIVATE_DER_BYTES).map_err(|_| Error::InvalidKey)?;
    // Parse borrowed DER first, before big-integer validation/precomputation.
    let info =
        rsa::pkcs8::PrivateKeyInfo::try_from(der.as_slice()).map_err(|_| Error::InvalidKey)?;
    let parts =
        rsa::pkcs1::RsaPrivateKey::try_from(info.private_key).map_err(|_| Error::InvalidKey)?;
    check_public_parts(parts.modulus.as_bytes(), parts.public_exponent.as_bytes())?;
    if parts.other_prime_infos.is_some()
        || [
            parts.private_exponent,
            parts.prime1,
            parts.prime2,
            parts.exponent1,
            parts.exponent2,
            parts.coefficient,
        ]
        .iter()
        .any(|part| part.as_bytes().len() > RSA_BYTES)
    {
        return Err(Error::InvalidKey);
    }
    let key = RsaPrivateKey::from_pkcs8_der(&der).map_err(|_| Error::InvalidKey)?;
    // Reject ignored optional fields or inconsistent CRT values as well as
    // alternate encodings; only the exact canonical persistence form is valid.
    if private_der(&key)?.as_slice() != der.as_slice() {
        return Err(Error::InvalidKey);
    }
    Ok(RecipientKey { key })
}

pub fn recipient_key_digest(encoded: &str) -> Result<[u8; 32], Error> {
    let (_, der) = decode_public_key(encoded)?;
    Ok(Sha256::digest(&*der).into())
}

pub fn seal(
    plaintext: &[u8],
    public_key: &str,
    binding: &managed::Binding,
) -> Result<Envelope, Error> {
    if plaintext.len() > MAX_PLAINTEXT_BYTES {
        return Err(Error::TooLarge);
    }
    let aad = aad(binding)?;
    let (recipient, _) = decode_public_key(public_key)?;
    let mut key = Zeroizing::new([0_u8; KEY_BYTES]);
    let mut nonce = [0_u8; NONCE_BYTES];
    OsRng
        .try_fill_bytes(key.as_mut())
        .map_err(|_| Error::Cryptography)?;
    OsRng
        .try_fill_bytes(&mut nonce)
        .map_err(|_| Error::Cryptography)?;
    let wrapped_key = recipient
        .encrypt(&mut OsRng, Oaep::new_with_label::<Sha256, _>(DOMAIN), &*key)
        .map_err(|_| Error::Cryptography)?;
    let cipher = Aes256Gcm::new_from_slice(&*key).map_err(|_| Error::Cryptography)?;
    let mut ciphertext = Zeroizing::new(plaintext.to_vec());
    let tag = cipher
        .encrypt_in_place_detached(Nonce::from_slice(&nonce), &aad, &mut ciphertext)
        .map_err(|_| Error::Cryptography)?;
    ciphertext.extend_from_slice(&tag);
    Ok(Envelope {
        version: VERSION,
        wrapped_key: URL_SAFE_NO_PAD.encode(wrapped_key),
        nonce: URL_SAFE_NO_PAD.encode(nonce),
        ciphertext: URL_SAFE_NO_PAD.encode(&*ciphertext),
    })
}

pub fn open(
    envelope: &Envelope,
    recipient: &RecipientKey,
    binding: &managed::Binding,
) -> Result<Zeroizing<Vec<u8>>, Error> {
    if envelope.version != VERSION {
        return Err(Error::InvalidEnvelope);
    }
    let aad = aad(binding)?;
    // Every field is bounded and canonical before any private RSA operation.
    let wrapped_key = decode_b64(&envelope.wrapped_key, RSA_BYTES, RSA_BYTES)?;
    let nonce = decode_b64(&envelope.nonce, NONCE_BYTES, NONCE_BYTES)?;
    let mut plaintext = decode_b64(
        &envelope.ciphertext,
        TAG_BYTES,
        MAX_PLAINTEXT_BYTES + TAG_BYTES,
    )?;
    let key = Zeroizing::new(
        recipient
            .key
            .decrypt_blinded(
                &mut OsRng,
                Oaep::new_with_label::<Sha256, _>(DOMAIN),
                &wrapped_key,
            )
            .map_err(|_| Error::Cryptography)?,
    );
    if key.len() != KEY_BYTES {
        return Err(Error::Cryptography);
    }
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| Error::Cryptography)?;
    let length = plaintext.len() - TAG_BYTES;
    let tag = Tag::clone_from_slice(&plaintext[length..]);
    plaintext.truncate(length);
    cipher
        .decrypt_in_place_detached(Nonce::from_slice(&nonce), &aad, &mut plaintext, &tag)
        .map_err(|_| Error::Cryptography)?;
    Ok(plaintext)
}

fn aad(binding: &managed::Binding) -> Result<Vec<u8>, Error> {
    // UUID parsing is bounded before context_field allocates its JSON string.
    if [
        &binding.node_id,
        &binding.sandbox_uid,
        &binding.pod_uid,
        &binding.writer_fence,
        &binding.operation_id,
        &binding.delivery_id,
    ]
    .iter()
    .any(|id| id.len() > 45)
    {
        return Err(Error::InvalidBinding);
    }
    binding.validate().map_err(|_| Error::InvalidBinding)?;
    let context = binding.context_field();
    let mut aad = Vec::with_capacity(DOMAIN.len() + 5 + context.len());
    aad.extend_from_slice(DOMAIN.as_bytes());
    aad.push(0);
    aad.extend_from_slice(&VERSION.to_be_bytes());
    aad.extend_from_slice(context.as_bytes());
    Ok(aad)
}

fn check_public_parts(modulus: &[u8], exponent: &[u8]) -> Result<(), Error> {
    if modulus.len() != RSA_BYTES
        || modulus[0] & 0x80 == 0
        || modulus[RSA_BYTES - 1] & 1 == 0
        || exponent != [1, 0, 1]
    {
        return Err(Error::InvalidKey);
    }
    Ok(())
}

fn decode_public_key(encoded: &str) -> Result<(RsaPublicKey, Zeroizing<Vec<u8>>), Error> {
    let der =
        decode_b64(encoded, PUBLIC_DER_BYTES, PUBLIC_DER_BYTES).map_err(|_| Error::InvalidKey)?;
    let key = RsaPublicKey::from_public_key_der(&der).map_err(|_| Error::InvalidKey)?;
    check_public_parts(&key.n().to_bytes_be(), &key.e().to_bytes_be())?;
    let canonical = key.to_public_key_der().map_err(|_| Error::InvalidKey)?;
    if canonical.as_bytes() != der.as_slice() {
        return Err(Error::InvalidKey);
    }
    Ok((key, der))
}

fn decode_b64(encoded: &str, min: usize, max: usize) -> Result<Zeroizing<Vec<u8>>, Error> {
    let encoded_len = |bytes: usize| (bytes * 4).div_ceil(3);
    if encoded.len() < encoded_len(min) || encoded.len() > encoded_len(max) {
        return Err(Error::InvalidEnvelope);
    }
    // decode_slice keeps even partial private-key decoding in a zeroizing
    // allocation when the decoder rejects a malformed final character.
    let mut bytes = Zeroizing::new(vec![0_u8; max]);
    let length = URL_SAFE_NO_PAD
        .decode_slice(encoded, &mut bytes)
        .map_err(|_| Error::InvalidEnvelope)?;
    if length < min || length > max {
        return Err(Error::InvalidEnvelope);
    }
    bytes.truncate(length);
    // URL_SAFE_NO_PAD already rejects padding, wrong alphabets, and nonzero
    // trailing bits, so no second secret-bearing encoded allocation is needed.
    Ok(bytes)
}

fn private_der(key: &RsaPrivateKey) -> Result<Zeroizing<Vec<u8>>, Error> {
    // Keep both the inner PKCS#1 and outer PKCS#8 encodings zeroizing. Building
    // the DER explicitly avoids a non-zeroizing intermediate private Vec.
    let modulus = key.n().to_bytes_be();
    let exponent = key.e().to_bytes_be();
    let d = Zeroizing::new(key.d().to_bytes_be());
    let p = Zeroizing::new(key.primes()[0].to_bytes_be());
    let q = Zeroizing::new(key.primes()[1].to_bytes_be());
    let dp = Zeroizing::new(key.dp().ok_or(Error::InvalidKey)?.to_bytes_be());
    let dq = Zeroizing::new(key.dq().ok_or(Error::InvalidKey)?.to_bytes_be());
    let coefficient = Zeroizing::new(key.crt_coefficient().ok_or(Error::InvalidKey)?);
    let coefficient_bytes = Zeroizing::new(coefficient.to_bytes_be());
    let uint = |bytes| rsa::pkcs1::UintRef::new(bytes).map_err(|_| Error::InvalidKey);
    let parts = rsa::pkcs1::RsaPrivateKey {
        modulus: uint(&modulus)?,
        public_exponent: uint(&exponent)?,
        private_exponent: uint(&d)?,
        prime1: uint(&p)?,
        prime2: uint(&q)?,
        exponent1: uint(&dp)?,
        exponent2: uint(&dq)?,
        coefficient: uint(&coefficient_bytes)?,
        other_prime_infos: None,
    };
    let inner = Zeroizing::new(parts.to_der().map_err(|_| Error::InvalidKey)?);
    let outer = Zeroizing::new(
        rsa::pkcs8::PrivateKeyInfo::new(rsa::pkcs1::ALGORITHM_ID, &inner)
            .to_der()
            .map_err(|_| Error::InvalidKey)?,
    );
    if outer.len() > MAX_PRIVATE_DER_BYTES {
        return Err(Error::InvalidKey);
    }
    Ok(outer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use rsa::{
        BigUint, RsaPublicKey,
        pkcs8::{DecodePublicKey, EncodePublicKey},
        traits::PublicKeyParts,
    };
    use std::sync::OnceLock;

    // Fresh disposable interoperability fixture. Never used outside these tests.
    const PRIVATE_KEY: &str = "MIIG_QIBADANBgkqhkiG9w0BAQEFAASCBucwggbjAgEAAoIBgQCghmohJ4oi46MIqYiZ0TBshGnxuP-muv5iCnbhQAVfTi214E9q4TooFekWI2KQhdrEQDeuCroGsbPhw6a7dkBdr9mGoCsf8T2c4cvftIoxwzWqOHHGVkmUY2NZok88othNYUI1nEbA9OY9y7BcgP4wgvLrql5K0eJ21ISqLbHhQEEcTfcFZ7g6mYKeAubrTliN8Wd6K-Hc6bnpXfmylE-0LFGhcVoQ1CJBKB89sjz33sdsNyw2UOq11ugfUDxcCagxmwKebAQWW6ki4z30nE0vB40ShmeBKl2_mRTkRgMQ4bUO_ZdmVETRPEdHe1kRmoZxeHjnidL6SRx9LSeWQmQXUz-VoCemXFbzgXAHWXNMGA-XO8dzsULoKPJFmUGVwDmq4Jcj-0aftIUmYX7KXHSitfwg5_c0lUgil38Zlfqwk4qmDpFMDONgBfnNRsTkz-eVtw59ur9LRM97DTBfegoZCYos-RZN27j78JjVtKuN11-u1t75_mYmCzeu9uqaVUcCAwEAAQKCAYAMu1yDkK1GIyP1Ne0dthfPI5l7kJ7GTKKMiCPxrbA_ZOX_PPFPmtmQAFziYqAYsmUtl6JS7YrfqQyJZYk96R0l0SLBqMaC3BF09fvDnIAxfjgtbhioEIP5fwCMRXFfb9MUIBiuQ9zgxQvV7ufoDlsa4ef-hR7BdHAv2J9FGCa5pO0FH7Ht_dIA49gWTEigdTBU4cbBS-W4LdmPBuMdJOJFehZnZaQtHYYyAMqpjA-F4WeTz7R0Amr7TTZP9X80FZffODJCDPKxp_lLZHk53GuZ1miZcdxEnY_BtUT6ycox-Xj9j-wS_c-s_rRDacraICChR284Y9axC1MlirRHHkQP73pwmDwe5OXfj6Hhz4XGB28RJB2BN6PMLRfNlXF5RF_IZXNZueMCDAVD7raDjbypr1RFtGl8bKUf6COEw6wSunNtnfdAuGjnw4y8wKm7AZWiQAEIjW1qw-H9kyiq9qMrxiPa4ao7Az_lOyiwmEag_nBjgCY8i7IVyoeXfaZpmZECgcEA32l4sm7oJRT7aPZSRAgj-L6iVpQC7L95fneea30-uF3lzeAJn2SqEBLMgDQR0LrKyFXH1Vd7BS9ct6gk5kpX_GzvEc0R20OzqJ9ZG_GYCBz2Zr3_ALOpOu61QnzLo2bZqD4O_xtsH3tyhd_-RuhFUIZTocNMLv1QH-x68gSoa6QMvIlohMIqNqc7xiOpHyIisgq9lpuJxpF7fWF4VLXP1ziMkT7UzI6BdAuQm1W4zJCLTWBH-xBIi_fgGjA7K2TdAoHBALfwp0MzJg7kOxqpz-V9iFmMEm_IveUyCgQCsnSCyLD71TY8TbUc-tmrb9bWrjB5jUvzrtByBPo4xFu9EMgTE5TEHeH07wcIP8kV1ENL5zTAol7swpJ0PfuW1CzpfATGqF6Sll5WT-payx5jonuPwQe4kxJEhfHjJWQ3_0ia1aXVSIWr7NtnAtSjXyh1oJDRfizHC27uDzFOVDbw2oLF5TtZbj3IYpFIvV4tbuBksw_TnZ41_LYkdMyTUsDw8kC-cwKBwGN5FqB7-Sjpmrc1VZOs_Dc4yhc6d8LpzjKdGBBMB60XQcbUxfQOYhoeBKfghz4krT1d1EgNV10LpYCwgR14ZIykBB4s7JJwJoIh0Q88FrEmFB_S4MNzOxpeRp5_6jurA6aBWzQDBbVOxcMN5K21pYchCAOybS1O5GoSZPt1T3f95x3bIIpb2WfUNi0Wm6qdDE2xspBoWIPtKsq8p1MwUIZHS28wRpd9q-37aDuBg3KxuRSOlf4XVfXcPA_tAatkkQKBwQCqaXW9J20SDNbJPBwU5HlnZQNLtzJfXj2zO4BzuxsOBTKifboMmp24_kCuTNSyqlqCzG_gRFJiNCePSpl6k1nn0M5Soia1YCogNW4jLvY0hGeqBDW7k6A9L57BQUuZ7Sbd_3b6TYLaf2WBcRN-4E8YxSLUoNFrqWLh95M2g-R-MBxxYzgciNIxiG5HwLqSkNQRzhLEoh9HMeeJwxjeYVOf6Znd-YGAUmDHGNaJZCIU_pwfkKrhYOMpCTB4-oGmBAsCgcBLKs8U8bLM9pdAndOEB63l91AEoxRBg6SXHzJbt79ycoRSggIzfpjiGXm5jAeIJ7yL0ogVTe8_cymMSBiE3VFGXw1JH55Bvn4PEp90zuG8vI1raVwjg-oX7MOkrozo8Zg03WVUOyImlbCDZer0vIB6WQNWFfMAkXx0ZBpyEPfeKW6JFhZcVkbtdpOxW5eoNE3fLHHCgAvMNzKfMap42by3_x2hPlMBcFQYfUvfanPeiA-C75RD0HNJR7NhY4zHq2w";
    const PUBLIC_KEY: &str = "MIIBojANBgkqhkiG9w0BAQEFAAOCAY8AMIIBigKCAYEAoIZqISeKIuOjCKmImdEwbIRp8bj_prr-Ygp24UAFX04tteBPauE6KBXpFiNikIXaxEA3rgq6BrGz4cOmu3ZAXa_ZhqArH_E9nOHL37SKMcM1qjhxxlZJlGNjWaJPPKLYTWFCNZxGwPTmPcuwXID-MILy66peStHidtSEqi2x4UBBHE33BWe4OpmCngLm605YjfFneivh3Om56V35spRPtCxRoXFaENQiQSgfPbI8997HbDcsNlDqtdboH1A8XAmoMZsCnmwEFlupIuM99JxNLweNEoZngSpdv5kU5EYDEOG1Dv2XZlRE0TxHR3tZEZqGcXh454nS-kkcfS0nlkJkF1M_laAnplxW84FwB1lzTBgPlzvHc7FC6CjyRZlBlcA5quCXI_tGn7SFJmF-ylx0orX8IOf3NJVIIpd_GZX6sJOKpg6RTAzjYAX5zUbE5M_nlbcOfbq_S0TPew0wX3oKGQmKLPkWTdu4-_CY1bSrjddfrtbe-f5mJgs3rvbqmlVHAgMBAAE";

    fn recipient() -> &'static RecipientKey {
        static KEY: OnceLock<RecipientKey> = OnceLock::new();
        KEY.get_or_init(|| decode_private_key(PRIVATE_KEY).unwrap())
    }

    fn other_recipient() -> &'static RecipientKey {
        static KEY: OnceLock<RecipientKey> = OnceLock::new();
        KEY.get_or_init(|| generate_recipient().unwrap())
    }

    fn binding() -> managed::Binding {
        managed::Binding {
            node_id: "11111111-1111-4111-8111-111111111111".into(),
            sandbox_uid: "22222222-2222-4222-8222-222222222222".into(),
            pod_uid: "33333333-3333-4333-8333-333333333333".into(),
            runtime_generation: 7,
            credential_epoch: 11,
            writer_fence: "44444444-4444-4444-8444-444444444444".into(),
            operation_id: "55555555-5555-4555-8555-555555555555".into(),
            delivery_id: "66666666-6666-4666-8666-666666666666".into(),
        }
    }

    fn flipped(value: &str) -> String {
        let mut bytes = URL_SAFE_NO_PAD.decode(value).unwrap();
        bytes[0] ^= 1;
        URL_SAFE_NO_PAD.encode(bytes)
    }

    #[test]
    fn round_trip_and_private_persistence_preserve_recipient() {
        let public = recipient().public_key_b64u().unwrap();
        assert_eq!(public, PUBLIC_KEY);
        let stored = encode_private_key(recipient()).unwrap();
        assert_eq!(stored.as_str(), PRIVATE_KEY);
        let restored = decode_private_key(&stored).unwrap();
        let envelope = seal(b"bootstrap credential", &public, &binding()).unwrap();
        let encoded = serde_json::to_string(&envelope).unwrap();
        let decoded: Envelope = serde_json::from_str(&encoded).unwrap();
        assert_eq!(envelope, decoded);
        assert_eq!(
            open(&decoded, &restored, &binding()).unwrap().as_slice(),
            b"bootstrap credential"
        );
        assert_eq!(
            recipient_key_digest(&public).unwrap(),
            sha2::Sha256::digest(URL_SAFE_NO_PAD.decode(&public).unwrap()).as_slice()
        );
    }

    #[test]
    fn generated_recipient_uses_fixed_parameters_and_wrong_recipient_fails() {
        let public = other_recipient().public_key_b64u().unwrap();
        let key =
            RsaPublicKey::from_public_key_der(&URL_SAFE_NO_PAD.decode(&public).unwrap()).unwrap();
        assert_eq!(key.n().bits(), 3072);
        assert_eq!(key.e(), &BigUint::from(65537_u32));
        let envelope = seal(b"bootstrap credential", PUBLIC_KEY, &binding()).unwrap();
        assert!(open(&envelope, other_recipient(), &binding()).is_err());
    }

    #[test]
    fn each_binding_field_is_authenticated() {
        let expected = binding();
        let envelope = seal(b"bootstrap credential", PUBLIC_KEY, &expected).unwrap();
        let replacement = "77777777-7777-4777-8777-777777777777";
        let mut cases = Vec::new();
        macro_rules! change {
            ($field:ident, $value:expr) => {{
                let mut changed = expected.clone();
                changed.$field = $value;
                cases.push((stringify!($field), changed));
            }};
        }
        change!(node_id, replacement.into());
        change!(sandbox_uid, replacement.into());
        change!(pod_uid, replacement.into());
        change!(runtime_generation, expected.runtime_generation + 1);
        change!(credential_epoch, expected.credential_epoch + 1);
        change!(writer_fence, replacement.into());
        change!(operation_id, replacement.into());
        change!(delivery_id, replacement.into());
        assert_eq!(cases.len(), 8);
        for (field, changed) in cases {
            assert!(
                open(&envelope, recipient(), &changed).is_err(),
                "unbound {field}"
            );
        }
    }

    #[test]
    fn invalid_binding_is_rejected_on_both_paths() {
        let envelope = seal(b"credential", PUBLIC_KEY, &binding()).unwrap();
        for invalid in [0, 1, 2] {
            let mut changed = binding();
            match invalid {
                0 => changed.runtime_generation = 0,
                1 => changed.credential_epoch = 0,
                _ => changed.node_id = "x".repeat(4096),
            }
            assert!(seal(b"credential", PUBLIC_KEY, &changed).is_err());
            assert!(open(&envelope, recipient(), &changed).is_err());
        }
    }

    #[test]
    fn wrapped_key_nonce_ciphertext_and_version_tampering_fail() {
        let envelope = seal(b"bootstrap credential", PUBLIC_KEY, &binding()).unwrap();
        let mut variants = Vec::new();
        let mut changed = envelope.clone();
        changed.version = 2;
        variants.push(changed);
        let mut changed = envelope.clone();
        changed.wrapped_key = flipped(&changed.wrapped_key);
        variants.push(changed);
        let mut changed = envelope.clone();
        changed.nonce = flipped(&changed.nonce);
        variants.push(changed);
        let mut changed = envelope.clone();
        changed.ciphertext = flipped(&changed.ciphertext);
        variants.push(changed);
        for changed in variants {
            assert!(open(&changed, recipient(), &binding()).is_err());
        }
    }

    #[test]
    fn field_size_and_encoding_limits_fail_closed() {
        let envelope = seal(b"credential", PUBLIC_KEY, &binding()).unwrap();
        for length in [0, 383, 385, 2048] {
            let mut changed = envelope.clone();
            changed.wrapped_key = URL_SAFE_NO_PAD.encode(vec![0; length]);
            assert!(open(&changed, recipient(), &binding()).is_err());
        }
        for length in [0, 11, 13, 2048] {
            let mut changed = envelope.clone();
            changed.nonce = URL_SAFE_NO_PAD.encode(vec![0; length]);
            assert!(open(&changed, recipient(), &binding()).is_err());
        }
        for length in [0, 15, MAX_PLAINTEXT_BYTES + 17] {
            let mut changed = envelope.clone();
            changed.ciphertext = URL_SAFE_NO_PAD.encode(vec![0; length]);
            assert!(open(&changed, recipient(), &binding()).is_err());
        }
        for field in ["wrapped_key", "nonce", "ciphertext"] {
            let mut value = serde_json::to_value(&envelope).unwrap();
            value[field] = format!("{}=", value[field].as_str().unwrap()).into();
            let changed: Envelope = serde_json::from_value(value).unwrap();
            assert!(
                open(&changed, recipient(), &binding()).is_err(),
                "accepted padded {field}"
            );
        }
    }

    #[test]
    fn plaintext_limit_accepts_exact_boundary_and_rejects_next_byte() {
        for length in [0, MAX_PLAINTEXT_BYTES] {
            let plaintext = vec![0x5a; length];
            let envelope = seal(&plaintext, PUBLIC_KEY, &binding()).unwrap();
            assert_eq!(
                open(&envelope, recipient(), &binding()).unwrap().as_slice(),
                plaintext
            );
            assert!(serde_json::to_vec(&envelope).unwrap().len() < 65536);
        }
        assert!(seal(&vec![0; MAX_PLAINTEXT_BYTES + 1], PUBLIC_KEY, &binding()).is_err());
    }

    #[test]
    fn public_keys_reject_noncanonical_old_formats_sizes_and_exponents() {
        for value in [
            String::new(),
            format!("{PUBLIC_KEY}="),
            format!("{PUBLIC_KEY}\n"),
            URL_SAFE_NO_PAD.encode([7; 32]),
            "A".repeat(4096),
        ] {
            assert!(recipient_key_digest(&value).is_err());
            assert!(seal(b"credential", &value, &binding()).is_err());
        }
        for (bits, exponent) in [
            (2048, 65537_u32),
            (3071, 65537),
            (4096, 65537),
            (3072, 3),
            (3072, 65539),
        ] {
            let modulus = (BigUint::from(1_u8) << (bits - 1)) + BigUint::from(1_u8);
            let public = RsaPublicKey::new(modulus, BigUint::from(exponent)).unwrap();
            let encoded = URL_SAFE_NO_PAD.encode(public.to_public_key_der().unwrap().as_bytes());
            assert!(
                recipient_key_digest(&encoded).is_err(),
                "accepted RSA {bits} / {exponent}"
            );
        }
        let mut trailing = URL_SAFE_NO_PAD.decode(PUBLIC_KEY).unwrap();
        trailing.push(0);
        assert!(recipient_key_digest(&URL_SAFE_NO_PAD.encode(trailing)).is_err());
        let mut noncanonical = PUBLIC_KEY.as_bytes().to_vec();
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let last = noncanonical.last_mut().unwrap();
        let index = ALPHABET.iter().position(|c| c == last).unwrap();
        *last = ALPHABET[index | 1];
        assert!(recipient_key_digest(std::str::from_utf8(&noncanonical).unwrap()).is_err());
    }

    #[test]
    fn private_keys_reject_noncanonical_old_and_oversized_material() {
        for value in [
            String::new(),
            format!("{PRIVATE_KEY}="),
            format!("{PRIVATE_KEY}\n"),
            URL_SAFE_NO_PAD.encode([7; 32]),
            "A".repeat(4096),
        ] {
            assert!(decode_private_key(&value).is_err());
        }
        let mut trailing = URL_SAFE_NO_PAD.decode(PRIVATE_KEY).unwrap();
        trailing.push(0);
        assert!(decode_private_key(&URL_SAFE_NO_PAD.encode(trailing)).is_err());
    }

    #[test]
    fn private_keys_reject_wrong_parameters_and_inconsistent_crt_values() {
        use rsa::pkcs8::der::Encode;
        let der = URL_SAFE_NO_PAD.decode(PRIVATE_KEY).unwrap();
        let info = rsa::pkcs8::PrivateKeyInfo::try_from(der.as_slice()).unwrap();
        for field in ["exponent", "modulus", "private_exponent", "coefficient"] {
            let mut parts = rsa::pkcs1::RsaPrivateKey::try_from(info.private_key).unwrap();
            let oversized = [0x55; 385];
            match field {
                "exponent" => parts.public_exponent = rsa::pkcs1::UintRef::new(&[1, 0, 3]).unwrap(),
                "modulus" => parts.modulus = rsa::pkcs1::UintRef::new(&[1; 383]).unwrap(),
                "private_exponent" => {
                    parts.private_exponent = rsa::pkcs1::UintRef::new(&oversized).unwrap()
                }
                _ => parts.coefficient = rsa::pkcs1::UintRef::new(&[1]).unwrap(),
            }
            let inner = parts.to_der().unwrap();
            let changed = rsa::pkcs8::PrivateKeyInfo::new(rsa::pkcs1::ALGORITHM_ID, &inner)
                .to_der()
                .unwrap();
            assert!(
                decode_private_key(&URL_SAFE_NO_PAD.encode(changed)).is_err(),
                "accepted changed {field}"
            );
        }
    }

    #[test]
    fn oaep_label_and_unwrapped_key_length_are_enforced() {
        use rand::rngs::OsRng;
        use rsa::Oaep;
        let public =
            RsaPublicKey::from_public_key_der(&URL_SAFE_NO_PAD.decode(PUBLIC_KEY).unwrap())
                .unwrap();
        let envelope = seal(b"credential", PUBLIC_KEY, &binding()).unwrap();
        for (label, length) in [
            ("other-domain", 32),
            ("nyxid-machine-bootstrap-v1", 31),
            ("nyxid-machine-bootstrap-v1", 33),
        ] {
            let mut changed = envelope.clone();
            changed.wrapped_key = URL_SAFE_NO_PAD.encode(
                public
                    .encrypt(
                        &mut OsRng,
                        Oaep::new_with_label::<sha2::Sha256, _>(label),
                        &vec![7; length],
                    )
                    .unwrap(),
            );
            assert!(open(&changed, recipient(), &binding()).is_err());
        }
    }

    #[test]
    fn wire_rejects_unknown_fields_and_obsolete_envelopes() {
        let envelope = seal(b"credential", PUBLIC_KEY, &binding()).unwrap();
        let mut value = serde_json::to_value(envelope).unwrap();
        value["ephemeral_public_key"] = "obsolete".into();
        assert!(serde_json::from_value::<Envelope>(value).is_err());
        assert!(
            serde_json::from_str::<Envelope>(
                r#"{"v":1,"epk":"obsolete","nonce":"old","ciphertext":"old"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn debug_and_errors_do_not_expose_keys_or_envelope_material() {
        let envelope = seal(b"credential", PUBLIC_KEY, &binding()).unwrap();
        assert_eq!(format!("{:?}", recipient()), "RecipientKey { [REDACTED] }");
        assert_eq!(format!("{envelope:?}"), "Envelope { [REDACTED] }");
        let error = decode_private_key("credential-value-must-not-leak").unwrap_err();
        assert!(!format!("{error:?}: {error}").contains("credential-value"));
    }

    #[test]
    fn aad_pins_domain_version_and_complete_ordered_binding() {
        let expected = concat!(
            "nyxid-machine-bootstrap-v1\0\0\0\0\x01",
            "{\"node_id\":\"11111111-1111-4111-8111-111111111111\",",
            "\"sandbox_uid\":\"22222222-2222-4222-8222-222222222222\",",
            "\"pod_uid\":\"33333333-3333-4333-8333-333333333333\",",
            "\"runtime_generation\":7,\"credential_epoch\":11,",
            "\"writer_fence\":\"44444444-4444-4444-8444-444444444444\",",
            "\"operation_id\":\"55555555-5555-4555-8555-555555555555\",",
            "\"delivery_id\":\"66666666-6666-4666-8666-666666666666\"}"
        );
        assert_eq!(aad(&binding()).unwrap(), expected.as_bytes());
    }

    #[test]
    fn shipping_python_fixtures_open_with_pinned_wire_digest_and_aad() {
        // These fixed envelopes were produced by the actual Python seal()
        // functions in both managed-node image fixtures, using this test key.
        for (wire, plaintext) in [
            (
                r#"{"version":1,"wrapped_key":"AB0_24INcCRFZZ5TcufsbiQVFbgz3ngFAg-jvOzrBO8KZ64WiqnjvrA-L61rvCk2XcLWdYXUa6qRHG_svKeLog3S1gT9_1D92x8AkuSNAbI-ytaWveAv1yPH-KMC6pkX47bOKH2ClNH9dHeHf_ixHbDNlQEKbLb0rctOkTYcbYrsVN6WaKKuihwGmpSdLO-dauQewdetiv2lDR6JF43-V-Kq6-u3w5_WPwFYDEE204h8-2k1sH_bTnU3yzo8jWgCf26Z3sEuQb-i3J8qvXMotJTRbIs92XV9_3Uausso9edAR3ZjxrxuS-HxCZalPqH9mRQLZQQSRi7sTNzeEdpd7oI6-i3754iQME-K2WPC48Nbs_kSMP3_ek5iptI9EoiyvES067RIk7h5E5UaFNMD7q4jMOnyAjpvq2b2QrAJwpYo_XnIZgok3rieKxcCDa8LCKv4YZM1D7FGYO_GybIUPfliOAI_FoJU8kHBnLF18JBvIeZV3BGMDqsSxg9uwjpQ","nonce":"tacVerHQzut2_bJx","ciphertext":"PY4q7UhVavlVhd7VJzyaENYIV5QozuZhOnQzLyTrUdV86hUsyfwgnXOK83n5d8HNAbzDhMhI_Japb_Q6bnJCreJQITPo8savaZzOCzZSt5CIrq9wizr0b7pbss4b8Uvc7FU2xIV7lNPasF2f5naPc7kOyZRZnbpQ10_TEtXw"}"#,
                r#"{"auth_token":"fixture-auth-token","signing_secret":"fixture-signing-secret","capabilities":["files","shell"]}"#,
            ),
            (
                r#"{"version":1,"wrapped_key":"PyIB6w7KUZNdWCxxrFDIPrZXN0V8YNLIsSySqrvWwcMQ9tDnbh-EApAo_ULADyCqrGv8zb2jKwEaCQ0tls-cgBdN_NJiCv3hKPuC8qiKeQeYBwUE0at_Vk7fGMEJ-eBCX35wjACxCp2HStj-YV8hKf1H6RQ7vx5X7y7jMo1UIjaoHkPBwclJa49kGUeS-7uqI54MUwWSB6pbUdG8p-kkYZ_BHFcrK-o7_3UgS6xjSNNyA1sFSu7L8yuryo5KjM4IgBQD7A0oxlzcP-_eZHAS2fJV-b2kR6C4OqiqjitqqGFtFA6zwbLiIWJ-O6Tt1ElVUT48jGMo_TiLeLw9aIzt2djjikHOTse11ym3wqeNNEJGI4mQdrOgSmkExZdijFurS56WQ54jekozXnBUH6Bctcxm4-dx1-yAiCnCRVsmgytUKmpafTYYVn_EEw4uiIMvmMhnvz0svgHyb1doJVHpWig6OdQkddvDPLNWc4AHRKETl_aqPYuIZ3I5NizmjtQ2","nonce":"hDXeM_5jjrvW4-vE","ciphertext":"RDKFhhmry1_DmCb1jVNXXOqcM55AvNmmj6mqYj3iERFMnigBkvg2xZYN9d0mr5ZK9FhdKC4B8tS8phWCbEG_sgt2B_qQoVWtgBRwSvXKrQoskYCa_K99tKpBENe6"}"#,
                r#"{"auth_token":"fixture-auth-token","signing_secret":"fixture-signing-secret"}"#,
            ),
        ] {
            let envelope: Envelope = serde_json::from_str(wire).unwrap();
            assert_eq!(serde_json::to_string(&envelope).unwrap(), wire);
            assert_eq!(
                open(&envelope, recipient(), &binding()).unwrap().as_slice(),
                plaintext.as_bytes()
            );
        }
        assert_eq!(
            hex::encode(recipient_key_digest(PUBLIC_KEY).unwrap()),
            "c6243a7871f642a3b7752e74be3c73833ccd2e3cfd5f9eef250f754694df16b5"
        );
        assert_eq!(
            hex::encode(aad(&binding()).unwrap()),
            "6e797869642d6d616368696e652d626f6f7473747261702d763100000000017b226e6f64655f6964223a2231313131313131312d313131312d343131312d383131312d313131313131313131313131222c2273616e64626f785f756964223a2232323232323232322d323232322d343232322d383232322d323232323232323232323232222c22706f645f756964223a2233333333333333332d333333332d343333332d383333332d333333333333333333333333222c2272756e74696d655f67656e65726174696f6e223a372c2263726564656e7469616c5f65706f6368223a31312c227772697465725f66656e6365223a2234343434343434342d343434342d343434342d383434342d343434343434343434343434222c226f7065726174696f6e5f6964223a2235353535353535352d353535352d343535352d383535352d353535353535353535353535222c2264656c69766572795f6964223a2236363636363636362d363636362d343636362d383636362d363636363636363636363636227d"
        );
    }
}
