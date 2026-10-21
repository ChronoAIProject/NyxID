//! Contextual envelopes. This format has no unbound decryption fallback.
use super::*;

const RECORD_MAGIC: &[u8; 4] = b"\x03NYX";
const RECORD_FIXED_HEADER: usize = 7;
const RECORD_DOMAIN: &[u8] = b"nyxid:record-envelope:v3\0";

/// Server-resolved identity of one encrypted field. Components are encoded
/// separately, so map keys, array identities and delimiter characters cannot
/// alias another record, owner or field. Never derive this from caller-supplied
/// ciphertext metadata.
pub struct RecordContext<'a> {
    collection: &'a str,
    record_id: &'a str,
    owner_id: &'a str,
    field: &'a [&'a str],
}

impl std::fmt::Debug for RecordContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordContext")
            .field("collection", &self.collection)
            .field("identity", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl<'a> RecordContext<'a> {
    pub fn new(
        collection: &'a str,
        record_id: &'a str,
        owner_id: &'a str,
        field: &'a [&'a str],
    ) -> Result<Self, AppError> {
        if collection.is_empty()
            || collection.len() > 128
            || !collection
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            || record_id.is_empty()
            || record_id.len() > 512
            || owner_id.is_empty()
            || owner_id.len() > 512
            || field.is_empty()
            || field.len() > 16
            || field
                .iter()
                .any(|part| part.is_empty() || part.len() > 1024)
        {
            return Err(AppError::Internal(
                "Invalid server encryption context".into(),
            ));
        }
        Ok(Self {
            collection,
            record_id,
            owner_id,
            field,
        })
    }

    fn authenticated_bytes(&self, header: &[u8]) -> Vec<u8> {
        let mut encoded = Vec::with_capacity(256 + header.len());
        encoded.extend_from_slice(RECORD_DOMAIN);
        for part in [self.collection, self.record_id, self.owner_id] {
            encode_component(&mut encoded, part.as_bytes());
        }
        encoded.extend_from_slice(&(self.field.len() as u32).to_be_bytes());
        for part in self.field {
            encode_component(&mut encoded, part.as_bytes());
        }
        // Includes format, key selector, exact wrapped-DEK bytes and lengths.
        encode_component(&mut encoded, header);
        encoded
    }
}

fn encode_component(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&(value.len() as u32).to_be_bytes());
    output.extend_from_slice(value);
}

fn invalid_record() -> AppError {
    AppError::Internal("Encrypted field authentication failed".into())
}

pub fn is_record_envelope(ciphertext: &[u8]) -> bool {
    ciphertext.starts_with(RECORD_MAGIC)
}

impl EncryptionKeys {
    pub(crate) fn key_version_retirement_evidence(
        &self,
    ) -> crate::crypto::key_provider::KeyVersionRetirementEvidence {
        self.provider.key_version_retirement_evidence()
    }

    /// Encrypt an exactly identified server field with a fresh DEK. Both the
    /// server context and complete public envelope header authenticate the data.
    pub async fn encrypt_record(
        &self,
        plaintext: &[u8],
        context: &RecordContext<'_>,
    ) -> Result<Vec<u8>, AppError> {
        if !self.contextual_record_writes {
            return Err(AppError::Conflict(
                "Encrypted field writes await the configured writer cutover".into(),
            ));
        }
        let mut dek = Zeroizing::new([0u8; 32]);
        rand::thread_rng().fill_bytes(dek.as_mut());
        let wrapped = self.provider.wrap_dek(dek.as_ref()).await?;
        if wrapped.ciphertext.is_empty() || wrapped.ciphertext.len() > MAX_WRAPPED_DEK_SIZE {
            return Err(invalid_record());
        }
        let mut result = Vec::with_capacity(
            RECORD_FIXED_HEADER
                + wrapped.ciphertext.len()
                + NONCE_SIZE
                + plaintext.len()
                + TAG_SIZE,
        );
        result.extend_from_slice(RECORD_MAGIC);
        result.push(wrapped.key_id);
        result.extend_from_slice(&(wrapped.ciphertext.len() as u16).to_be_bytes());
        result.extend_from_slice(&wrapped.ciphertext);
        let aad = context.authenticated_bytes(&result);
        let mut nonce = [0u8; NONCE_SIZE];
        rand::thread_rng().fill_bytes(&mut nonce);
        let cipher = Aes256Gcm::new_from_slice(dek.as_ref()).map_err(|_| invalid_record())?;
        let encrypted = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| invalid_record())?;
        result.extend_from_slice(&nonce);
        result.extend_from_slice(&encrypted);
        Ok(result)
    }

    /// Authenticate only this format and this exact context. An error never
    /// retries an old unbound decoder. Callers choose explicit legacy handling
    /// before invoking this method, according to their storage-version contract.
    pub async fn decrypt_record(
        &self,
        ciphertext: &[u8],
        context: &RecordContext<'_>,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.decrypt_record_with_policy(ciphertext, context, false)
            .await
    }

    /// Authenticate using only the selected primary provider and selector.
    /// An equal selector on a fallback does not establish this evidence. Cloud
    /// logical resources may decrypt historical internal versions: this is not
    /// version-destruction evidence; inspect key_version_retirement_evidence.
    pub(crate) async fn authenticate_current_record(
        &self,
        ciphertext: &[u8],
        context: &RecordContext<'_>,
    ) -> Result<(), AppError> {
        self.decrypt_record_with_policy(ciphertext, context, true)
            .await
            .map(drop)
    }

    async fn decrypt_record_with_policy(
        &self,
        ciphertext: &[u8],
        context: &RecordContext<'_>,
        primary_current_only: bool,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        if !is_record_envelope(ciphertext) || ciphertext.len() < RECORD_FIXED_HEADER {
            return Err(invalid_record());
        }
        if primary_current_only && ciphertext[4] != self.provider.current_key_id() {
            return Err(invalid_record());
        }
        let wrapped_len = u16::from_be_bytes([ciphertext[5], ciphertext[6]]) as usize;
        if wrapped_len == 0 || wrapped_len > MAX_WRAPPED_DEK_SIZE {
            return Err(invalid_record());
        }
        let header_len = RECORD_FIXED_HEADER + wrapped_len;
        if ciphertext.len() < header_len + NONCE_SIZE + TAG_SIZE {
            return Err(invalid_record());
        }
        let wrapped = WrappedKey {
            key_id: ciphertext[4],
            ciphertext: Zeroizing::new(ciphertext[RECORD_FIXED_HEADER..header_len].to_vec()),
        };
        // A configured old provider is an unwrap route for this exact format,
        // never permission to retry plaintext using a different AAD contract.
        let dek_bytes = match self.provider.unwrap_dek(&wrapped).await {
            Ok(dek) => dek,
            Err(_) if !primary_current_only => match &self.fallback_provider {
                Some(provider) if provider.has_key_id(wrapped.key_id) => provider
                    .unwrap_dek(&wrapped)
                    .await
                    .map_err(|_| invalid_record())?,
                _ => return Err(invalid_record()),
            },
            Err(_) => return Err(invalid_record()),
        };
        if dek_bytes.len() != 32 {
            return Err(invalid_record());
        }
        let aad = context.authenticated_bytes(&ciphertext[..header_len]);
        let payload = &ciphertext[header_len..];
        let cipher =
            Aes256Gcm::new_from_slice(dek_bytes.as_slice()).map_err(|_| invalid_record())?;
        let plaintext = cipher
            .decrypt(
                Nonce::from_slice(&payload[..NONCE_SIZE]),
                Payload {
                    msg: &payload[NONCE_SIZE..],
                    aad: &aad,
                },
            )
            .map_err(|_| invalid_record())?;
        Ok(Zeroizing::new(plaintext))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn context_debug_redacts_oauth_state_and_dynamic_binding_components() {
        let components = [
            "code_verifier",
            "distinctive-api-key-binding",
            "distinctive-runtime-handle",
        ];
        let context = super::RecordContext::new(
            "oauth_states",
            "distinctive-oauth-state-capability",
            "distinctive-owner-identity",
            &components,
        )
        .unwrap();
        let debug = format!("{context:?}");
        assert!(debug.contains("oauth_states"));
        for secret in [
            "distinctive-oauth-state-capability",
            "distinctive-owner-identity",
            "code_verifier",
            "distinctive-api-key-binding",
            "distinctive-runtime-handle",
        ] {
            assert!(!debug.contains(secret));
        }
        assert!(debug.contains("REDACTED"));
    }

    use super::*;
    use crate::test_utils::test_encryption_keys;

    #[tokio::test]
    async fn record_identity_and_header_are_authenticated_without_legacy_fallback() {
        let keys = test_encryption_keys();
        let context = RecordContext::new(
            "nodes",
            "record-a",
            "owner-a",
            &["signing_secret_encrypted"],
        )
        .unwrap();
        let blob = keys
            .encrypt_record(b"distinctive-record-secret", &context)
            .await
            .unwrap();
        assert_eq!(
            keys.decrypt_record(&blob, &context)
                .await
                .unwrap()
                .as_slice(),
            b"distinctive-record-secret"
        );
        for changed in [
            RecordContext::new(
                "other_nodes",
                "record-a",
                "owner-a",
                &["signing_secret_encrypted"],
            )
            .unwrap(),
            RecordContext::new(
                "nodes",
                "record-b",
                "owner-a",
                &["signing_secret_encrypted"],
            )
            .unwrap(),
            RecordContext::new(
                "nodes",
                "record-a",
                "owner-b",
                &["signing_secret_encrypted"],
            )
            .unwrap(),
            RecordContext::new("nodes", "record-a", "owner-a", &["other_secret"]).unwrap(),
        ] {
            assert!(keys.decrypt_record(&blob, &changed).await.is_err());
        }
        for index in [0, 3, 4, 5, 6, 7, blob.len() - 1] {
            let mut changed = blob.clone();
            changed[index] ^= 1;
            assert!(
                keys.decrypt_record(&changed, &context).await.is_err(),
                "header/payload byte {index} not authenticated"
            );
        }
        let legacy = keys.encrypt(b"distinctive-record-secret").await.unwrap();
        assert!(keys.decrypt_record(&legacy, &context).await.is_err());
        assert!(keys.decrypt_with_aad(&blob, b"").await.is_err());
    }

    #[tokio::test]
    async fn context_components_cannot_alias_and_owner_rebind_requires_plaintext() {
        let keys = test_encryption_keys();
        let first =
            RecordContext::new("platform_credentials", "a:b", "c", &["secrets", "a.b"]).unwrap();
        let other =
            RecordContext::new("platform_credentials", "a", "b:c", &["secrets.a", "b"]).unwrap();
        let blob = keys.encrypt_record(b"payload", &first).await.unwrap();
        assert!(keys.decrypt_record(&blob, &other).await.is_err());
        let owner = RecordContext::new(
            "platform_credentials",
            "a:b",
            "successor",
            &["secrets", "a.b"],
        )
        .unwrap();
        let plaintext = keys.decrypt_record(&blob, &first).await.unwrap();
        let rebound = keys
            .encrypt_record(plaintext.as_slice(), &owner)
            .await
            .unwrap();
        assert!(keys.decrypt_record(&rebound, &first).await.is_err());
        assert_eq!(
            keys.decrypt_record(&rebound, &owner)
                .await
                .unwrap()
                .as_slice(),
            b"payload"
        );
        let mut oversized = blob;
        oversized[5..7].copy_from_slice(&1025u16.to_be_bytes());
        assert!(keys.decrypt_record(&oversized, &first).await.is_err());
    }
}
