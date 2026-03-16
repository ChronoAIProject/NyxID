use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Config error: {0}")]
    Config(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Encryption error: {0}")]
    Encryption(String),

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    #[error("Registration failed: {0}")]
    RegistrationFailed(String),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML serialization error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("TOML deserialization error: {0}")]
    TomlDeserialize(#[from] toml::de::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Keychain error: {0}")]
    Keychain(String),
}
