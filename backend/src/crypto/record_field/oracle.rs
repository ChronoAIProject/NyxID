//! The server authenticates the outer envelope of an already worker-sealed login.
use crate::crypto::aes::EncryptionKeys;
use crate::errors::AppResult;
use crate::models::oracle_login_profile::OracleLoginProfile;
use zeroize::Zeroizing;

pub(crate) struct OracleOuter(String);

impl OracleOuter {
    pub(crate) fn profile(row: &OracleLoginProfile) -> Self {
        Self(format!(
            "oracle-login-profile:{}:{}:{}:{}:v{}",
            row.pool_id, row.id, row.generation, row.revision, row.format_version
        ))
    }

    pub(crate) fn snapshot(pool: &str, snapshot: &str, format: u32) -> Self {
        Self(format!("oracle-login-snapshot:{pool}:{snapshot}:v{format}"))
    }

    pub(crate) async fn seal(&self, keys: &EncryptionKeys, sealed: &[u8]) -> AppResult<Vec<u8>> {
        keys.encrypt_with_aad(sealed, self.0.as_bytes()).await
    }

    pub(crate) async fn open(
        &self,
        keys: &EncryptionKeys,
        ciphertext: &[u8],
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        keys.decrypt_with_aad(ciphertext, self.0.as_bytes()).await
    }

    pub(crate) async fn authenticate_current(
        &self,
        keys: &EncryptionKeys,
        ciphertext: &[u8],
    ) -> AppResult<Option<usize>> {
        keys.authenticated_current_v2_size(ciphertext, self.0.as_bytes())
            .await
    }

    pub(crate) async fn rewrap(
        &self,
        keys: &EncryptionKeys,
        ciphertext: &[u8],
    ) -> AppResult<Vec<u8>> {
        keys.rewrap_with_aad(ciphertext, self.0.as_bytes()).await
    }
}
