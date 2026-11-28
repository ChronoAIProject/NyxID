#[allow(dead_code)]
fn reviewed_receivers() {
    use super::EncryptionKeys;
    let _ = EncryptionKeys::seal_census_record;
    let _ = EncryptionKeys::open_census_record;
    let _ = EncryptionKeys::encrypt;
    let _ = EncryptionKeys::decrypt;
    let _ = EncryptionKeys::encrypt_with_aad;
    let _ = EncryptionKeys::decrypt_with_aad;
    let _ = EncryptionKeys::rewrap;
    let _ = EncryptionKeys::rewrap_with_aad;
    let _ = crate::crypto::record_field::authenticated::observe;
    #[cfg(test)]
    let _ = (super::encrypt, super::decrypt);
}
