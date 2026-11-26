use super::*;
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};

#[test]
fn historical_identity_encodings_include_empty_and_delimited_components() {
    let digest = "ab01".repeat(16);
    assert_eq!(
        HistoricalAad::broker_binding(&digest).unwrap().as_bytes(),
        digest.as_bytes()
    );
    let receipt = format!("sha256:{digest}");
    assert_eq!(
        HistoricalAad::native_reply(&receipt).unwrap().as_bytes(),
        receipt.as_bytes()
    );
    assert!(HistoricalAad::native_reply(&digest).is_none());
    assert!(HistoricalAad::broker_binding(&receipt).is_none());
    for bad in ["", "abc", &digest.to_uppercase(), &"g".repeat(64)] {
        assert!(HistoricalAad::broker_binding(bad).is_none());
    }
    assert_eq!(
        HistoricalAad::erasure_capability("", "λ:").as_bytes(),
        b"nyxid:erasure-capability:v1\0\x00\x00\x00\x18personal_erasure_effects\x00\x00\x00\x00\x00\x00\x00\x03\xce\xbb:\x00\x00\x00\x14capability_encrypted",
    );
    assert_eq!(
        HistoricalAad::mobile_delivery("λ/%2F", "", ":user").as_bytes(),
        "mobile-social-delivery-v1:λ/%2F:::user".as_bytes()
    );
}

#[tokio::test]
async fn historical_aad_opens_authenticated_v0_with_oversized_v2_length() {
    let keys = crate::test_utils::test_encryption_keys();
    let nonce = *b"\x02\x01\xff\xffnonce123";
    let plain = [0x51; 64];
    let historical = HistoricalAad::mobile_delivery("hash", "pkce", "user");
    let cipher = Aes256Gcm::new_from_slice(&[0x11; 32]).unwrap();
    let mut stored = nonce.to_vec();
    stored.extend(
        cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: historical.as_bytes(),
                },
            )
            .unwrap(),
    );
    assert_eq!(
        historical.open(&keys, &stored).await.unwrap().as_slice(),
        plain
    );
    let swapped = HistoricalAad::mobile_delivery("hash", "other", "user");
    assert!(swapped.open(&keys, &stored).await.is_err());
}
