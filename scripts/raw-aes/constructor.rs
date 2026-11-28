#[allow(dead_code)]
fn raw_aes_receiver_control(keys: &crate::crypto::aes::EncryptionKeys, key: &[u8]) {
    use aes_gcm::{Aes256Gcm as Cipher, KeyInit as _};
    use crate::crypto::aes::EncryptionKeys as Keys;
    use crate::crypto::record_field::authenticated::observe as read;
    let _ = read; // nyxid_server::crypto::record_field::authenticated::observe
    let _ = Keys::seal_census_record; // nyxid_server::crypto::aes::EncryptionKeys::seal_census_record
    drop(keys.open_census_record("generation", &["fixture"], key)); // nyxid_server::crypto::aes::EncryptionKeys::open_census_record
    let _ = lopdf::Document::encrypt; // lopdf::Document::encrypt
    drop(keys.decrypt(key)); // nyxid_server::crypto::aes::EncryptionKeys::decrypt
    let _ = Cipher::new_from_slice(key); // aes_gcm::aead::KeyInit::new_from_slice
    let _ = aes_gcm::Aes256Gcm::new_from_slice(key); // aes_gcm::aead::KeyInit::new_from_slice
}
