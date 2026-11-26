//! Server-encrypted binary fields whose historical format used unbound encryption.
//! Services supply trusted identities and interpret the stored field version.
//! Fields with historical AAD use their owning adapter. Recipient-sealed
//! deliveries use the recipient protocol and key.
use super::aes::EncryptionKeys;
pub(crate) use super::aes::RecordContext;
use crate::errors::{AppError, AppResult};
use zeroize::Zeroizing;

pub(crate) async fn encrypt(
    keys: &EncryptionKeys,
    collection: &str,
    id: &str,
    owner: &str,
    field: &str,
    bytes: &[u8],
) -> AppResult<Vec<u8>> {
    encrypt_context(
        keys,
        &RecordContext::new(collection, id, owner, &[field])?,
        bytes,
    )
    .await
}

pub(crate) async fn encrypt_context(
    keys: &EncryptionKeys,
    context: &RecordContext<'_>,
    bytes: &[u8],
) -> AppResult<Vec<u8>> {
    if keys.contextual_record_writes() {
        keys.encrypt_record(bytes, context).await
    } else {
        keys.encrypt(bytes).await
    }
}

pub(crate) async fn decrypt(
    keys: &EncryptionKeys,
    collection: &str,
    id: &str,
    owner: &str,
    field: &str,
    version: Option<u8>,
    ciphertext: &[u8],
) -> AppResult<Zeroizing<Vec<u8>>> {
    decrypt_context(
        keys,
        &RecordContext::new(collection, id, owner, &[field])?,
        version,
        ciphertext,
    )
    .await
}

pub(crate) async fn decrypt_context(
    keys: &EncryptionKeys,
    context: &RecordContext<'_>,
    version: Option<u8>,
    ciphertext: &[u8],
) -> AppResult<Zeroizing<Vec<u8>>> {
    if version == Some(3) {
        keys.decrypt_record(ciphertext, context).await
    } else if version.is_none() && keys.allow_legacy_record_reads() {
        keys.decrypt(ciphertext).await
    } else {
        Err(AppError::Internal(
            "Record field requires contextual migration".into(),
        ))
    }
}

pub(crate) async fn verify_current(
    keys: &EncryptionKeys,
    collection: &str,
    id: &str,
    owner: &str,
    field: &str,
    version: Option<u8>,
    ciphertext: &[u8],
) -> AppResult<()> {
    if version != Some(3) {
        return Err(AppError::Internal(
            "Record field requires contextual migration".into(),
        ));
    }
    keys.authenticate_current_record(
        ciphertext,
        &RecordContext::new(collection, id, owner, &[field])?,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn explicit_dispatch_accepts_authenticated_v0_magic_nonce_and_never_downgrades_v3() {
        use aes_gcm::{
            Aes256Gcm, Nonce,
            aead::{Aead, KeyInit},
        };
        let mut keys = crate::test_utils::test_encryption_keys();
        let nonce = *b"\x03NYXnonce123";
        let cipher = Aes256Gcm::new_from_slice(&[0x11; 32]).unwrap();
        let mut legacy = nonce.to_vec();
        legacy.extend(
            cipher
                .encrypt(
                    Nonce::from_slice(&nonce),
                    b"distinctive-legacy-material".as_slice(),
                )
                .unwrap(),
        );
        assert!(legacy.starts_with(b"\x03NYX"));
        assert_eq!(
            decrypt(
                &keys,
                "assistant_attachments",
                "record",
                "owner",
                "text_encrypted",
                None,
                &legacy
            )
            .await
            .unwrap()
            .as_slice(),
            b"distinctive-legacy-material"
        );
        assert!(
            decrypt(
                &keys,
                "assistant_attachments",
                "record",
                "owner",
                "text_encrypted",
                Some(3),
                &legacy
            )
            .await
            .is_err()
        );
        let contextual = encrypt(
            &keys,
            "assistant_attachments",
            "record",
            "owner",
            "text_encrypted",
            b"current-material",
        )
        .await
        .unwrap();
        assert!(
            decrypt(
                &keys,
                "assistant_attachments",
                "record",
                "owner",
                "text_encrypted",
                Some(9),
                &contextual
            )
            .await
            .is_err()
        );
        let mut corrupt = contextual.clone();
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(
            decrypt(
                &keys,
                "assistant_attachments",
                "record",
                "owner",
                "text_encrypted",
                Some(3),
                &corrupt
            )
            .await
            .is_err()
        );
        keys.set_allow_legacy_record_reads(false);
        assert!(
            decrypt(
                &keys,
                "assistant_attachments",
                "record",
                "owner",
                "text_encrypted",
                None,
                &legacy
            )
            .await
            .is_err()
        );
        assert_eq!(
            decrypt(
                &keys,
                "assistant_attachments",
                "record",
                "owner",
                "text_encrypted",
                Some(3),
                &contextual
            )
            .await
            .unwrap()
            .as_slice(),
            b"current-material"
        );
    }
}
