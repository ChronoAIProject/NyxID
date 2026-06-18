use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

type HmacSha256 = Hmac<Sha256>;

pub fn derive_hmac_key(
    label: &str,
    encryption_key: Option<&[u8]>,
    jwt_private_pem: &[u8],
) -> Zeroizing<[u8; 32]> {
    if let Some(master) = encryption_key {
        let label = format!("nyxid:{label}-code-hmac-v1");
        return hmac_key(master, label.as_bytes());
    }

    if !jwt_private_pem.is_empty() {
        let label = format!("nyxid:{label}-code-hmac-v1:jwt");
        return hmac_key(jwt_private_pem, label.as_bytes());
    }

    panic!("HMAC key has no source: encryption key and JWT private key are both missing");
}

fn hmac_key(secret: &[u8], info: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC-SHA256 accepts any key length");
    mac.update(info);
    let digest = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Zeroizing::new(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_hmac_key_domain_separates_labels() {
        let encryption_key = [0x42_u8; 32];
        let jwt_private_pem = [0x99_u8; 512];

        let cli = derive_hmac_key("cli-pairing", Some(&encryption_key), &jwt_private_pem);
        let auth = derive_hmac_key("auth-device", Some(&encryption_key), &jwt_private_pem);

        assert_ne!(cli.as_slice(), auth.as_slice());
    }

    #[test]
    fn derive_hmac_key_falls_back_to_jwt_pem() {
        let jwt_private_pem = [0x99_u8; 512];

        let a = derive_hmac_key("auth-device", None, &jwt_private_pem);
        let b = derive_hmac_key("auth-device", None, &jwt_private_pem);

        assert_eq!(a.as_slice(), b.as_slice());
    }
}
