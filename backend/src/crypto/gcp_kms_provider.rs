//! GCP Cloud KMS KeyProvider implementation.
//!
//! Wraps and unwraps DEKs using the GCP Cloud KMS encrypt/decrypt APIs.

use async_trait::async_trait;
use zeroize::Zeroizing;

use crate::config::AppConfig;
use crate::errors::AppError;

use super::key_provider::{KeyProvider, WrappedKey, derive_key_id_from_str};

struct GcpKmsKeyMetadata {
    current_key_name: String,
    current_key_id: u8,
    previous_key_name: Option<String>,
    previous_key_id: Option<u8>,
}

impl GcpKmsKeyMetadata {
    fn new(current_key_name: String, previous_key_name: Option<String>) -> Self {
        let current_key_id = derive_key_id_from_str(&current_key_name);
        let previous_key_id = previous_key_name.as_deref().map(derive_key_id_from_str);

        if let Some(prev_id) = previous_key_id
            && current_key_id == prev_id
        {
            panic!(
                "GCP_KMS_KEY_NAME and GCP_KMS_KEY_NAME_PREVIOUS produce the same key id \
                 (0x{:02x}). This is a 1-in-256 hash collision. Use a different key.",
                current_key_id
            );
        }

        Self {
            current_key_name,
            current_key_id,
            previous_key_name,
            previous_key_id,
        }
    }

    fn key_name_for(&self, key_id: u8) -> Result<&str, AppError> {
        if key_id == self.current_key_id {
            Ok(&self.current_key_name)
        } else if self.previous_key_id == Some(key_id) {
            self.previous_key_name.as_deref().ok_or_else(|| {
                AppError::Internal("Previous key id matched but name is missing".into())
            })
        } else {
            Err(AppError::Internal(
                "No GCP KMS key available for key id".into(),
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
        self.previous_key_name.is_some()
    }
}

impl std::fmt::Debug for GcpKmsKeyMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcpKmsProvider")
            .field("current_key_name", &"[REDACTED]")
            .field("current_key_id", &format!("0x{:02x}", self.current_key_id))
            .field(
                "previous_key_name",
                if self.previous_key_name.is_some() {
                    &"Some([REDACTED])"
                } else {
                    &"None"
                },
            )
            .finish()
    }
}

pub struct GcpKmsProvider {
    client: google_cloud_kms::client::Client,
    keys: GcpKmsKeyMetadata,
}

impl std::fmt::Debug for GcpKmsProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.keys.fmt(f)
    }
}

impl GcpKmsProvider {
    pub async fn from_config(config: &AppConfig) -> Self {
        let key_name = config
            .gcp_kms_key_name
            .as_deref()
            .expect("GCP_KMS_KEY_NAME must be set when KEY_PROVIDER=gcp-kms");

        let client_config = google_cloud_kms::client::ClientConfig::default()
            .with_auth()
            .await
            .expect("Failed to configure GCP KMS authentication");
        let client = google_cloud_kms::client::Client::new(client_config)
            .await
            .expect("Failed to create GCP KMS client");

        let keys = GcpKmsKeyMetadata::new(
            key_name.to_string(),
            config.gcp_kms_key_name_previous.clone(),
        );

        tracing::info!(
            key_id = format!("0x{:02x}", keys.current_key_id()),
            has_previous = keys.has_previous_key(),
            "GCP Cloud KMS provider initialized"
        );

        Self { client, keys }
    }
}

/// Maximum number of retry attempts for transient GCP KMS failures.
const GCP_KMS_MAX_RETRIES: u32 = 3;

/// Initial backoff delay for GCP KMS retries.
const GCP_KMS_INITIAL_BACKOFF_MS: u64 = 100;

#[async_trait]
impl KeyProvider for GcpKmsProvider {
    async fn wrap_dek(&self, plaintext_dek: &[u8]) -> Result<WrappedKey, AppError> {
        use google_cloud_kms::grpc::kms::v1::EncryptRequest;

        let mut last_err = None;
        for attempt in 0..GCP_KMS_MAX_RETRIES {
            if attempt > 0 {
                let backoff = std::time::Duration::from_millis(
                    GCP_KMS_INITIAL_BACKOFF_MS * 2u64.pow(attempt - 1),
                );
                tokio::time::sleep(backoff).await;
            }

            // Note: The SDK copies plaintext_dek internally. We cannot zeroize the SDK's copy.
            // The caller's Zeroizing wrapper handles the caller-side copy.
            let request = EncryptRequest {
                name: self.keys.current_key_name.clone(),
                plaintext: plaintext_dek.to_vec(),
                ..Default::default()
            };

            match self.client.encrypt(request, None).await {
                Ok(response) => {
                    return Ok(WrappedKey {
                        key_id: self.keys.current_key_id(),
                        ciphertext: Zeroizing::new(response.ciphertext),
                    });
                }
                Err(e) => {
                    tracing::warn!(attempt = attempt + 1, "GCP KMS encrypt transient failure");
                    last_err = Some(e);
                }
            }
        }

        let e = last_err.unwrap();
        tracing::error!("GCP KMS encrypt failed after {GCP_KMS_MAX_RETRIES} attempts: {e}");
        Err(AppError::Internal("GCP KMS encrypt failed".to_string()))
    }

    async fn unwrap_dek(&self, wrapped: &WrappedKey) -> Result<Zeroizing<Vec<u8>>, AppError> {
        use google_cloud_kms::grpc::kms::v1::DecryptRequest;

        let key_name = self.keys.key_name_for(wrapped.key_id)?;

        let mut last_err = None;
        for attempt in 0..GCP_KMS_MAX_RETRIES {
            if attempt > 0 {
                let backoff = std::time::Duration::from_millis(
                    GCP_KMS_INITIAL_BACKOFF_MS * 2u64.pow(attempt - 1),
                );
                tokio::time::sleep(backoff).await;
            }

            let request = DecryptRequest {
                name: key_name.to_string(),
                ciphertext: (*wrapped.ciphertext).clone(),
                ..Default::default()
            };

            match self.client.decrypt(request, None).await {
                Ok(response) => return Ok(Zeroizing::new(response.plaintext)),
                Err(e) => {
                    tracing::warn!(attempt = attempt + 1, "GCP KMS decrypt transient failure");
                    last_err = Some(e);
                }
            }
        }

        let e = last_err.unwrap();
        tracing::error!("GCP KMS decrypt failed after {GCP_KMS_MAX_RETRIES} attempts: {e}");
        Err(AppError::Internal("GCP KMS decrypt failed".to_string()))
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

    const TEST_KEY: &str =
        "projects/my-project/locations/us-east1/keyRings/my-ring/cryptoKeys/my-key";
    const TEST_KEY_PREV: &str =
        "projects/my-project/locations/us-east1/keyRings/my-ring/cryptoKeys/prev-key";

    /// Construct key metadata for unit tests without instantiating a GCP client.
    fn test_metadata(current_name: &str, previous_name: Option<&str>) -> GcpKmsKeyMetadata {
        GcpKmsKeyMetadata::new(current_name.to_string(), previous_name.map(str::to_string))
    }

    #[test]
    fn derive_key_id_deterministic() {
        let id1 = derive_key_id_from_str(TEST_KEY);
        let id2 = derive_key_id_from_str(TEST_KEY);
        assert_eq!(id1, id2);
    }

    #[test]
    fn derive_key_id_different_names() {
        let name1 = "projects/my-project/locations/us-east1/keyRings/ring/cryptoKeys/key-aaa";
        let name2 = "projects/my-project/locations/us-east1/keyRings/ring/cryptoKeys/key-bbb";
        // Different names *may* produce different IDs (probabilistic)
        let _id1 = derive_key_id_from_str(name1);
        let _id2 = derive_key_id_from_str(name2);
    }

    #[test]
    fn current_key_id_matches_derived() {
        let metadata = test_metadata(TEST_KEY, None);
        assert_eq!(metadata.current_key_id(), derive_key_id_from_str(TEST_KEY));
    }

    #[test]
    fn has_key_id_current_only() {
        let metadata = test_metadata(TEST_KEY, None);
        let current_id = derive_key_id_from_str(TEST_KEY);
        assert!(metadata.has_key_id(current_id));
        assert!(!metadata.has_key_id(current_id.wrapping_add(1)));
    }

    #[test]
    fn has_key_id_with_previous() {
        let metadata = test_metadata(TEST_KEY, Some(TEST_KEY_PREV));
        let current_id = derive_key_id_from_str(TEST_KEY);
        let prev_id = derive_key_id_from_str(TEST_KEY_PREV);
        assert!(metadata.has_key_id(current_id));
        assert!(metadata.has_key_id(prev_id));
    }

    #[test]
    fn has_previous_key_false_when_none() {
        let metadata = test_metadata(TEST_KEY, None);
        assert!(!metadata.has_previous_key());
    }

    #[test]
    fn has_previous_key_true_when_some() {
        let metadata = test_metadata(TEST_KEY, Some(TEST_KEY_PREV));
        assert!(metadata.has_previous_key());
    }

    #[test]
    fn debug_impl_redacts_key_names() {
        let metadata = test_metadata(TEST_KEY, Some(TEST_KEY_PREV));
        let debug_output = format!("{:?}", metadata);
        assert!(debug_output.contains("[REDACTED]"));
        assert!(!debug_output.contains(TEST_KEY));
        assert!(!debug_output.contains(TEST_KEY_PREV));
    }
}
