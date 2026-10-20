use thiserror::Error;

#[derive(Debug, Error)]
pub enum CloudAuthError {
    #[error("invalid credential payload: {0}")]
    InvalidCredential(&'static str),

    #[error("signing failed: {0}")]
    Signing(&'static str),

    #[error("network error: {0}")]
    Network(&'static str),
}

pub type CloudAuthResult<T> = Result<T, CloudAuthError>;

impl From<reqwest::Error> for CloudAuthError {
    fn from(err: reqwest::Error) -> Self {
        CloudAuthError::Network(if err.is_timeout() {
            "timeout"
        } else if err.is_connect() {
            "connection_failed"
        } else {
            "transport_failed"
        })
    }
}

impl From<serde_json::Error> for CloudAuthError {
    fn from(_: serde_json::Error) -> Self {
        CloudAuthError::InvalidCredential("malformed_json")
    }
}
