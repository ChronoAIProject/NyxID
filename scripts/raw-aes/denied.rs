// Each marked expression must fail at its own span in the real backend crate.
use crate::crypto::{aes::EncryptionKeys as Keys, record_field::authenticated};
#[allow(deprecated, unused_imports)]
pub(crate) use authenticated::observe as published; // deprecated observe

#[allow(dead_code)]
fn rejected(pair: (&Keys, &[u8])) {
    let (key, blob) = pair;
    let _ = key.encrypt(blob); // E0624 encrypt
    let _ = Keys::decrypt(key, blob); // E0624 decrypt
    let _item = Keys::encrypt_with_aad; // E0624 encrypt_with_aad
    macro_rules! open {
        ($key:expr) => { $key.decrypt_with_aad(blob, blob) }; // E0624 decrypt_with_aad
    }
    let _ = open!(key);
    let _ = key.rewrap(blob); // E0624 rewrap
    let _ = key.rewrap_with_aad(blob, blob); // E0624 rewrap_with_aad
    let _ = Keys::seal_census_record; // deprecated seal_census_record
    let _ = key.open_census_record("generation", &["field"], blob); // deprecated open_census_record
    let _ = authenticated::observe; // deprecated observe
    use authenticated::observe as read; // deprecated observe
    let _ = read; // deprecated observe

    #[cfg(test)]
    {
        use crate::crypto::aes::encrypt as seal; // test E0603 encrypt
        let _ = seal;
        let _ = crate::crypto::aes::decrypt; // test E0603 decrypt
    }
    #[cfg(not(test))]
    {
        let _ = crate::crypto::aes::encrypt; // normal E0425 encrypt
        let _ = crate::crypto::aes::decrypt; // normal E0425 decrypt
    }
}
