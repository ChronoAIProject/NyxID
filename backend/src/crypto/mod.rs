pub mod aes;
pub mod apple_client_secret;
pub mod device_code;
pub mod dpop;
pub mod hmac_keys;
pub(crate) mod hmac_lifecycle;
pub mod jwks;
pub mod jwt;
#[cfg(test)]
mod jwt_verifier_inventory_tests;
pub mod key_provider;
pub mod local_key_provider;
pub mod mtls;
pub mod password;
pub(crate) mod secret_json;
pub mod telegram;
pub mod token;

#[cfg(feature = "aws-kms")]
pub mod aws_kms_provider;

#[cfg(feature = "gcp-kms")]
pub mod gcp_kms_provider;

/// Transfer a plaintext allocation to its UTF-8 owner, including cleanup on error.
pub(crate) fn guarded_utf8(
    mut bytes: zeroize::Zeroizing<Vec<u8>>,
) -> Result<zeroize::Zeroizing<String>, std::str::Utf8Error> {
    use zeroize::{Zeroize, Zeroizing};
    String::from_utf8(std::mem::take(&mut *bytes))
        .map(Zeroizing::new)
        .map_err(|invalid| {
            let error = invalid.utf8_error();
            invalid.into_bytes().zeroize();
            error
        })
}

#[cfg(test)]
mod tests {
    use super::guarded_utf8;
    use zeroize::Zeroizing;

    #[test]
    fn guarded_utf8_reuses_the_allocation_and_reports_only_the_error_position() {
        let bytes = Zeroizing::new("dummy-credential".as_bytes().to_vec());
        let address = bytes.as_ptr();
        let text = guarded_utf8(bytes).unwrap();
        assert_eq!(text.as_ptr(), address);
        assert_eq!(text.as_str(), "dummy-credential");

        let mut invalid = Zeroizing::new(text.as_bytes().to_vec());
        invalid.push(0xff);
        let error = guarded_utf8(invalid).unwrap_err();
        assert_eq!(error.valid_up_to(), text.len());
        assert!(!format!("{error} {error:?}").contains(text.as_str()));
    }
}
