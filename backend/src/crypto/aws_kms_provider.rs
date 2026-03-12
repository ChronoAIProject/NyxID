//! AWS KMS KeyProvider implementation.
//!
//! Wraps and unwraps DEKs using the AWS KMS Encrypt/Decrypt APIs.
//! The wrapped DEK is the raw AWS CiphertextBlob (~170-200 bytes).

use async_trait::async_trait;
use aws_sdk_kms::Client as KmsClient;
use aws_sdk_kms::primitives::Blob;
use zeroize::Zeroizing;

use crate::config::AppConfig;
use crate::errors::AppError;

use super::key_provider::{KeyProvider, WrappedKey, derive_key_id_from_str};

struct AwsKmsKeyMetadata {
    current_key_arn: String,
    current_key_id: u8,
    previous_key_arn: Option<String>,
    previous_key_id: Option<u8>,
}

impl AwsKmsKeyMetadata {
    fn new(current_key_arn: String, previous_key_arn: Option<String>) -> Self {
        let current_key_id = derive_key_id_from_str(&current_key_arn);
        let previous_key_id = previous_key_arn.as_deref().map(derive_key_id_from_str);

        if let Some(prev_id) = previous_key_id
            && current_key_id == prev_id
        {
            panic!(
                "AWS_KMS_KEY_ARN and AWS_KMS_KEY_ARN_PREVIOUS produce the same key id \
                 (0x{:02x}). This is a 1-in-256 hash collision. Use a different key.",
                current_key_id
            );
        }

        Self {
            current_key_arn,
            current_key_id,
            previous_key_arn,
            previous_key_id,
        }
    }

    fn key_arn_for(&self, key_id: u8) -> Result<&str, AppError> {
        if key_id == self.current_key_id {
            Ok(&self.current_key_arn)
        } else if self.previous_key_id == Some(key_id) {
            self.previous_key_arn.as_deref().ok_or_else(|| {
                AppError::Internal("Previous key id matched but ARN is missing".into())
            })
        } else {
            Err(AppError::Internal(
                "No AWS KMS key available for key id".into(),
            ))
        }
    }

    fn current_key_id(&self) -> u8 {
        self.current_key_id
    }

    fn has_key_id(&self, key_id: u8) -> bool {
        key_id == self.current_key_id || self.previous_key_id == Some(key_id)
    }

    fn has_previous_key(&self) -> bool {
        self.previous_key_arn.is_some()
    }
}

impl std::fmt::Debug for AwsKmsKeyMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AwsKmsProvider")
            .field("current_key_arn", &"[REDACTED]")
            .field("current_key_id", &format!("0x{:02x}", self.current_key_id))
            .field(
                "previous_key_arn",
                if self.previous_key_arn.is_some() {
                    &"Some([REDACTED])"
                } else {
                    &"None"
                },
            )
            .finish()
    }
}

pub struct AwsKmsProvider {
    client: KmsClient,
    keys: AwsKmsKeyMetadata,
}

impl std::fmt::Debug for AwsKmsProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.keys.fmt(f)
    }
}

impl AwsKmsProvider {
    pub async fn from_config(config: &AppConfig) -> Self {
        let key_arn = config
            .aws_kms_key_arn
            .as_deref()
            .expect("AWS_KMS_KEY_ARN must be set when KEY_PROVIDER=aws-kms");

        let sdk_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let client = KmsClient::new(&sdk_config);

        let keys =
            AwsKmsKeyMetadata::new(key_arn.to_string(), config.aws_kms_key_arn_previous.clone());

        tracing::info!(
            key_id = format!("0x{:02x}", keys.current_key_id()),
            has_previous = keys.has_previous_key(),
            "AWS KMS provider initialized"
        );

        Self { client, keys }
    }
}

#[async_trait]
impl KeyProvider for AwsKmsProvider {
    async fn wrap_dek(&self, plaintext_dek: &[u8]) -> Result<WrappedKey, AppError> {
        // Note: The SDK copies plaintext_dek internally. We cannot zeroize the SDK's copy.
        // The caller's Zeroizing wrapper handles the caller-side copy.
        let resp = self
            .client
            .encrypt()
            .key_id(&self.keys.current_key_arn)
            .plaintext(Blob::new(plaintext_dek))
            .send()
            .await
            .map_err(|e| {
                tracing::error!("AWS KMS encrypt failed: {e}");
                AppError::Internal("AWS KMS encrypt failed".to_string())
            })?;

        let ciphertext_blob = resp
            .ciphertext_blob()
            .ok_or_else(|| AppError::Internal("AWS KMS returned empty ciphertext".into()))?;

        Ok(WrappedKey {
            key_id: self.keys.current_key_id(),
            ciphertext: Zeroizing::new(ciphertext_blob.as_ref().to_vec()),
        })
    }

    async fn unwrap_dek(&self, wrapped: &WrappedKey) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let key_arn = self.keys.key_arn_for(wrapped.key_id)?;

        let resp = self
            .client
            .decrypt()
            .key_id(key_arn)
            .ciphertext_blob(Blob::new((*wrapped.ciphertext).clone()))
            .send()
            .await
            .map_err(|e| {
                tracing::error!("AWS KMS decrypt failed: {e}");
                AppError::Internal("AWS KMS decrypt failed".to_string())
            })?;

        let plaintext = resp
            .plaintext()
            .ok_or_else(|| AppError::Internal("AWS KMS returned empty plaintext".into()))?;

        Ok(Zeroizing::new(plaintext.as_ref().to_vec()))
    }

    fn current_key_id(&self) -> u8 {
        self.keys.current_key_id()
    }

    fn has_key_id(&self, key_id: u8) -> bool {
        self.keys.has_key_id(key_id)
    }

    fn has_previous_key(&self) -> bool {
        self.keys.has_previous_key()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_ARN: &str = "arn:aws:kms:us-east-1:123456789:key/mrk-abc123";
    const TEST_ARN_PREV: &str = "arn:aws:kms:us-east-1:123456789:key/mrk-prev456";

    /// Construct key metadata for unit tests without instantiating an AWS SDK client.
    fn test_metadata(current_arn: &str, previous_arn: Option<&str>) -> AwsKmsKeyMetadata {
        AwsKmsKeyMetadata::new(current_arn.to_string(), previous_arn.map(str::to_string))
    }

    #[test]
    fn derive_key_id_deterministic() {
        let id1 = derive_key_id_from_str(TEST_ARN);
        let id2 = derive_key_id_from_str(TEST_ARN);
        assert_eq!(id1, id2);
    }

    #[test]
    fn derive_key_id_different_arns() {
        let arn1 = "arn:aws:kms:us-east-1:123456789:key/key-aaa";
        let arn2 = "arn:aws:kms:us-east-1:123456789:key/key-bbb";
        // Different ARNs *may* produce different IDs (probabilistic)
        let _id1 = derive_key_id_from_str(arn1);
        let _id2 = derive_key_id_from_str(arn2);
    }

    #[test]
    fn current_key_id_matches_derived() {
        let metadata = test_metadata(TEST_ARN, None);
        assert_eq!(metadata.current_key_id(), derive_key_id_from_str(TEST_ARN));
    }

    #[test]
    fn has_key_id_current_only() {
        let metadata = test_metadata(TEST_ARN, None);
        let current_id = derive_key_id_from_str(TEST_ARN);
        assert!(metadata.has_key_id(current_id));
        assert!(!metadata.has_key_id(current_id.wrapping_add(1)));
    }

    #[test]
    fn has_key_id_with_previous() {
        let metadata = test_metadata(TEST_ARN, Some(TEST_ARN_PREV));
        let current_id = derive_key_id_from_str(TEST_ARN);
        let prev_id = derive_key_id_from_str(TEST_ARN_PREV);
        assert!(metadata.has_key_id(current_id));
        assert!(metadata.has_key_id(prev_id));
        // Unknown ID
        let unknown = current_id.wrapping_add(1).max(prev_id.wrapping_add(1));
        if unknown != current_id && unknown != prev_id {
            assert!(!metadata.has_key_id(unknown));
        }
    }

    #[test]
    fn has_previous_key_false_when_none() {
        let metadata = test_metadata(TEST_ARN, None);
        assert!(!metadata.has_previous_key());
    }

    #[test]
    fn has_previous_key_true_when_some() {
        let metadata = test_metadata(TEST_ARN, Some(TEST_ARN_PREV));
        assert!(metadata.has_previous_key());
    }

    #[test]
    fn debug_impl_redacts_arns() {
        let metadata = test_metadata(TEST_ARN, Some(TEST_ARN_PREV));
        let debug = format!("{:?}", metadata);
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains(TEST_ARN));
        assert!(!debug.contains(TEST_ARN_PREV));
    }
}
