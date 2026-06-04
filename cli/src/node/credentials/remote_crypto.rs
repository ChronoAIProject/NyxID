use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use zeroize::Zeroizing;

use super::crypto::{
    PENDING_CREDENTIAL_DECRYPT_FAILED_CODE, PENDING_CREDENTIAL_VERSION_UNSUPPORTED_CODE,
    PendingCredentialCiphertext, PendingCredentialCryptoMetadata, RemoteCredentialCryptoOutbound,
};
use super::pending_key_store::PendingCredentialKeyStore;
use crate::node::config::NodeConfig;
use crate::node::credential_store::{CredentialStore, SharedCredentialsSender};
use crate::node::error::{Error, Result};
use crate::node::secret_backend::SecretBackend;

pub fn prepare_pubkey(
    config: &mut NodeConfig,
    backend: &SecretBackend,
    metadata: &PendingCredentialCryptoMetadata,
) -> Result<RemoteCredentialCryptoOutbound> {
    if metadata.version != nyxid_crypto::VERSION_V1 {
        return Err(Error::Validation(format!(
            "Unsupported pending credential crypto version '{}'",
            metadata.version
        )));
    }
    let mut store = PendingCredentialKeyStore::new(config, backend);
    let public = store.create_or_load(metadata)?;
    Ok(RemoteCredentialCryptoOutbound::Pubkey {
        pending_id: public.pending_id,
        version: public.version,
        node_pubkey: public.node_pubkey,
    })
}

pub fn decrypt_and_store_ciphertext(
    config: &mut NodeConfig,
    config_path: &Path,
    backend: &SecretBackend,
    credential_sender: Option<&Arc<SharedCredentialsSender>>,
    message: &PendingCredentialCiphertext,
) -> RemoteCredentialCryptoOutbound {
    let result = decrypt_and_store_inner(config, config_path, backend, credential_sender, message);
    match result {
        Ok(()) => RemoteCredentialCryptoOutbound::DecryptResult {
            pending_id: message.pending_id.clone(),
            status: "ok".to_string(),
            error_code: None,
        },
        Err(RemoteCryptoFailure::UnsupportedVersion) => {
            RemoteCredentialCryptoOutbound::DecryptResult {
                pending_id: message.pending_id.clone(),
                status: "error".to_string(),
                error_code: Some(PENDING_CREDENTIAL_VERSION_UNSUPPORTED_CODE),
            }
        }
        Err(RemoteCryptoFailure::DecryptOrStore) => RemoteCredentialCryptoOutbound::DecryptResult {
            pending_id: message.pending_id.clone(),
            status: "error".to_string(),
            error_code: Some(PENDING_CREDENTIAL_DECRYPT_FAILED_CODE),
        },
    }
}

pub fn evict_pending_key(
    config: &mut NodeConfig,
    backend: &SecretBackend,
    pending_id: &str,
) -> Result<bool> {
    PendingCredentialKeyStore::new(config, backend).delete(pending_id)
}

pub fn sweep_expired_pending_keys(
    config: &mut NodeConfig,
    backend: &SecretBackend,
    now: DateTime<Utc>,
) -> Result<Vec<String>> {
    PendingCredentialKeyStore::new(config, backend).sweep_expired(now)
}

enum RemoteCryptoFailure {
    UnsupportedVersion,
    DecryptOrStore,
}

fn decrypt_and_store_inner(
    config: &mut NodeConfig,
    config_path: &Path,
    backend: &SecretBackend,
    credential_sender: Option<&Arc<SharedCredentialsSender>>,
    message: &PendingCredentialCiphertext,
) -> std::result::Result<(), RemoteCryptoFailure> {
    if message.version != nyxid_crypto::VERSION_V1 {
        return Err(RemoteCryptoFailure::UnsupportedVersion);
    }

    let entry = config
        .pending_crypto_keys
        .get(&message.pending_id)
        .cloned()
        .ok_or(RemoteCryptoFailure::DecryptOrStore)?;
    if entry.version != nyxid_crypto::VERSION_V1 {
        return Err(RemoteCryptoFailure::UnsupportedVersion);
    }

    let private_key = PendingCredentialKeyStore::new(config, backend)
        .load_private_key(&message.pending_id)
        .map_err(|error| {
            tracing::warn!(
                pending_credential_id = %message.pending_id,
                %error,
                "Failed to load pending credential private key"
            );
            RemoteCryptoFailure::DecryptOrStore
        })?;
    let context = nyxid_crypto::RciContext::new(
        config.node.id.clone(),
        message.pending_id.clone(),
        entry.service_slug.clone(),
        entry.injection_method.clone(),
        entry.field_name.clone(),
        entry.target_url.clone(),
        entry.version.clone(),
    );
    let envelope = nyxid_crypto::envelope_from_encoded_parts(
        message.version.clone(),
        &message.admin_pubkey,
        &message.nonce,
        &message.ciphertext,
    )
    .map_err(|error| {
        tracing::warn!(
            pending_credential_id = %message.pending_id,
            %error,
            "Invalid pending credential ciphertext envelope"
        );
        RemoteCryptoFailure::DecryptOrStore
    })?;
    let plaintext = nyxid_crypto::decrypt(&envelope, &private_key, &context).map_err(|error| {
        tracing::warn!(
            pending_credential_id = %message.pending_id,
            %error,
            "Pending credential decrypt failed"
        );
        RemoteCryptoFailure::DecryptOrStore
    })?;
    let secret = zeroizing_string_from_utf8(plaintext).map_err(|error| {
        tracing::warn!(
            pending_credential_id = %message.pending_id,
            %error,
            "Pending credential plaintext was not UTF-8"
        );
        RemoteCryptoFailure::DecryptOrStore
    })?;

    store_decrypted_credential(config, backend, &entry, secret.as_str()).map_err(|error| {
        tracing::warn!(
            pending_credential_id = %message.pending_id,
            %error,
            "Failed to store decrypted pending credential"
        );
        RemoteCryptoFailure::DecryptOrStore
    })?;
    PendingCredentialKeyStore::new(config, backend)
        .delete(&message.pending_id)
        .map_err(|error| {
            tracing::warn!(
                pending_credential_id = %message.pending_id,
                %error,
                "Failed to evict pending credential private key after decrypt"
            );
            RemoteCryptoFailure::DecryptOrStore
        })?;
    config.save(config_path).map_err(|error| {
        tracing::warn!(
            pending_credential_id = %message.pending_id,
            %error,
            "Failed to save config after pending credential decrypt"
        );
        RemoteCryptoFailure::DecryptOrStore
    })?;
    if let Some(sender) = credential_sender {
        let new_store =
            CredentialStore::from_config_with_backend(config, backend).map_err(|error| {
                tracing::warn!(
                    pending_credential_id = %message.pending_id,
                    %error,
                    "Failed to refresh credential store after pending credential decrypt"
                );
                RemoteCryptoFailure::DecryptOrStore
            })?;
        sender.update(new_store);
    }
    Ok(())
}

fn zeroizing_string_from_utf8(bytes: Zeroizing<Vec<u8>>) -> Result<Zeroizing<String>> {
    String::from_utf8(bytes.to_vec())
        .map(Zeroizing::new)
        .map_err(|error| Error::Encryption(format!("Decrypted credential is not UTF-8: {error}")))
}

fn store_decrypted_credential(
    config: &mut NodeConfig,
    backend: &SecretBackend,
    entry: &crate::node::config::PendingCryptoKeyConfig,
    secret: &str,
) -> Result<()> {
    match entry.injection_method.as_str() {
        "header" => config.add_header_credential_via(
            &entry.service_slug,
            &entry.field_name,
            secret,
            entry.target_url.as_deref(),
            backend,
        ),
        "query-param" | "query_param" => config.add_query_param_credential_via(
            &entry.service_slug,
            &entry.field_name,
            secret,
            entry.target_url.as_deref(),
            backend,
        ),
        "path-prefix" | "path_prefix" => config.add_path_prefix_credential_via(
            &entry.service_slug,
            &entry.field_name,
            secret,
            entry.target_url.as_deref(),
            backend,
        ),
        other => Err(Error::Validation(format!(
            "Unsupported injection method '{other}'"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::credential_store::CredentialInjection;
    use crate::node::encryption::LocalEncryption;

    fn file_backend(dir: &Path) -> SecretBackend {
        SecretBackend::File(LocalEncryption::load_or_generate(dir).unwrap())
    }

    fn config() -> NodeConfig {
        NodeConfig::new(
            "ws://localhost:3001/api/v1/nodes/ws".to_string(),
            "node-1".to_string(),
            "file".to_string(),
        )
    }

    fn metadata(field_name: &str) -> PendingCredentialCryptoMetadata {
        PendingCredentialCryptoMetadata {
            pending_id: "pending-1".to_string(),
            node_id: "node-1".to_string(),
            service_slug: "openai".to_string(),
            injection_method: "header".to_string(),
            field_name: field_name.to_string(),
            target_url: Some("https://api.openai.com/v1".to_string()),
            expires_at: "2099-01-01T00:00:00Z".to_string(),
            version: nyxid_crypto::VERSION_V1.to_string(),
        }
    }

    fn ciphertext_for(
        meta: &PendingCredentialCryptoMetadata,
        node_pubkey: &str,
        plaintext: &str,
    ) -> PendingCredentialCiphertext {
        let pubkey = nyxid_crypto::decode_b64u_array::<32>("node_pubkey", node_pubkey).unwrap();
        let envelope =
            nyxid_crypto::encrypt(plaintext.as_bytes(), pubkey, &meta.context()).unwrap();
        PendingCredentialCiphertext {
            pending_id: meta.pending_id.clone(),
            version: envelope.version.clone(),
            admin_pubkey: nyxid_crypto::encode_b64u(&envelope.admin_pubkey),
            nonce: nyxid_crypto::encode_b64u(&envelope.nonce),
            ciphertext: nyxid_crypto::encode_b64u(&envelope.ciphertext),
        }
    }

    fn pubkey_from(outbound: RemoteCredentialCryptoOutbound) -> String {
        match outbound {
            RemoteCredentialCryptoOutbound::Pubkey { node_pubkey, .. } => node_pubkey,
            other => panic!("expected pubkey, got {other:?}"),
        }
    }

    #[test]
    fn pubkey_generated_once_per_pending() {
        let dir = tempfile::tempdir().unwrap();
        let backend = file_backend(dir.path());
        let mut cfg = config();
        let meta = metadata("Authorization");

        let first = prepare_pubkey(&mut cfg, &backend, &meta).unwrap();
        let second = prepare_pubkey(&mut cfg, &backend, &meta).unwrap();

        assert_eq!(first, second);
        assert_eq!(cfg.pending_crypto_keys.len(), 1);
    }

    #[test]
    fn ciphertext_decrypt_stores_local_credential() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let backend = file_backend(dir.path());
        let mut cfg = config();
        let meta = metadata("Authorization");
        let node_pubkey = pubkey_from(prepare_pubkey(&mut cfg, &backend, &meta).unwrap());
        let message = ciphertext_for(&meta, &node_pubkey, "Bearer sk-rci");

        let result = decrypt_and_store_ciphertext(&mut cfg, &config_path, &backend, None, &message);

        assert_eq!(
            result,
            RemoteCredentialCryptoOutbound::DecryptResult {
                pending_id: "pending-1".to_string(),
                status: "ok".to_string(),
                error_code: None,
            }
        );
        assert!(!cfg.pending_crypto_keys.contains_key("pending-1"));
        let store = CredentialStore::from_config_with_backend(&cfg, &backend).unwrap();
        let credential = store.get("openai").unwrap();
        match &credential.injection {
            CredentialInjection::Header { name, value } => {
                assert_eq!(name, "Authorization");
                assert_eq!(value.as_str(), "Bearer sk-rci");
            }
            _ => panic!("expected header credential"),
        }
    }

    #[test]
    fn wrong_aad_reports_8006_without_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let backend = file_backend(dir.path());
        let mut cfg = config();
        let meta = metadata("Authorization");
        let wrong_meta = metadata("X-Api-Key");
        let node_pubkey = pubkey_from(prepare_pubkey(&mut cfg, &backend, &meta).unwrap());
        let message = ciphertext_for(&wrong_meta, &node_pubkey, "Bearer sk-secret-plaintext");

        let result = decrypt_and_store_ciphertext(&mut cfg, &config_path, &backend, None, &message);
        let debug = format!("{result:?}");

        assert_eq!(
            result,
            RemoteCredentialCryptoOutbound::DecryptResult {
                pending_id: "pending-1".to_string(),
                status: "error".to_string(),
                error_code: Some(PENDING_CREDENTIAL_DECRYPT_FAILED_CODE),
            }
        );
        assert!(!debug.contains("sk-secret-plaintext"));
        assert!(!cfg.credentials.contains_key("openai"));
    }

    #[test]
    fn version_unsupported_reports_8007() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let backend = file_backend(dir.path());
        let mut cfg = config();
        let meta = metadata("Authorization");
        let node_pubkey = pubkey_from(prepare_pubkey(&mut cfg, &backend, &meta).unwrap());
        let mut message = ciphertext_for(&meta, &node_pubkey, "Bearer sk-rci");
        message.version = "v2".to_string();

        let result = decrypt_and_store_ciphertext(&mut cfg, &config_path, &backend, None, &message);

        assert_eq!(
            result,
            RemoteCredentialCryptoOutbound::DecryptResult {
                pending_id: "pending-1".to_string(),
                status: "error".to_string(),
                error_code: Some(PENDING_CREDENTIAL_VERSION_UNSUPPORTED_CODE),
            }
        );
    }

    #[test]
    fn privkey_evicted_on_consume_decline_cancel_expire() {
        let dir = tempfile::tempdir().unwrap();
        let backend = file_backend(dir.path());
        let mut cfg = config();
        let meta = metadata("Authorization");

        prepare_pubkey(&mut cfg, &backend, &meta).unwrap();
        assert!(evict_pending_key(&mut cfg, &backend, "pending-1").unwrap());
        assert!(!evict_pending_key(&mut cfg, &backend, "pending-1").unwrap());

        prepare_pubkey(&mut cfg, &backend, &meta).unwrap();
        assert!(evict_pending_key(&mut cfg, &backend, "pending-1").unwrap());

        prepare_pubkey(&mut cfg, &backend, &meta).unwrap();
        assert!(evict_pending_key(&mut cfg, &backend, "pending-1").unwrap());

        let mut expiring = metadata("Authorization");
        expiring.pending_id = "pending-expired".to_string();
        expiring.expires_at = "2026-06-03T23:59:59Z".to_string();
        prepare_pubkey(&mut cfg, &backend, &expiring).unwrap();
        let now = DateTime::parse_from_rfc3339("2026-06-04T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            sweep_expired_pending_keys(&mut cfg, &backend, now).unwrap(),
            vec!["pending-expired".to_string()]
        );
    }
}
