use thiserror::Error;

pub type Result<T> = std::result::Result<T, RciCryptoError>;

#[derive(Debug, Error)]
pub enum RciCryptoError {
    #[error("field '{field}' exceeds 65535 bytes")]
    FieldTooLong { field: &'static str },

    #[error("base64url field '{field}' must not contain padding")]
    Base64Padding { field: &'static str },

    #[error("base64url field '{field}' failed to decode: {source}")]
    Base64Decode {
        field: &'static str,
        source: base64::DecodeError,
    },

    #[error("field '{field}' must decode to {expected} bytes, got {actual}")]
    InvalidLength {
        field: &'static str,
        expected: usize,
        actual: usize,
    },

    #[error("ciphertext exceeds maximum size: {actual} > {max}")]
    CiphertextTooLarge { actual: usize, max: usize },

    #[error("unsupported RCI crypto version '{0}'")]
    UnsupportedVersion(String),

    #[error("cryptographic operation failed")]
    Crypto,
}
