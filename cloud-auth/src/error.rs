use thiserror::Error;

#[derive(Debug, Error)]
pub enum CloudAuthError {
    #[error("invalid credential payload: {0}")]
    InvalidCredential(String),

    #[error("signing failed: {0}")]
    Signing(String),

    #[error("token mint failed: {0}")]
    TokenMint(String),

    #[error("network error: {0}")]
    Network(String),

    #[error("upstream returned {status}: {body}")]
    UpstreamError { status: u16, body: String },
}

pub type CloudAuthResult<T> = Result<T, CloudAuthError>;

impl From<reqwest::Error> for CloudAuthError {
    fn from(err: reqwest::Error) -> Self {
        CloudAuthError::Network(err.to_string())
    }
}

impl From<serde_json::Error> for CloudAuthError {
    fn from(err: serde_json::Error) -> Self {
        CloudAuthError::InvalidCredential(err.to_string())
    }
}
