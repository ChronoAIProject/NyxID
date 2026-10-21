//! Durable selector outcomes; provider handles and payloads stay off receipts.
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct UploadSource {
    /// Original turn; late commits can only be cleaned up, never adopted.
    #[serde(default)]
    pub turn_id: String,
    pub event_key: String,
    pub index: usize,
    pub embedded: bool,
    pub declared: usize,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MediaOutcome {
    pub event_id: String,
    pub event_key: String,
    /// Absent for omitted items or an unavailable source event.
    pub index: Option<usize>,
    pub embedded: bool,
    pub status: String,
    pub upload_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MediaBound {
    pub event_ids: Vec<String>,
    pub ok: Vec<MediaOutcome>,
    pub failed: Vec<MediaOutcome>,
    pub context: String,
}

macro_rules! redacted {
    ($($name:ident),+) => {$ (
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(concat!(stringify!($name), "([REDACTED])"))
            }
        }
    )+};
}
redacted!(UploadSource, MediaOutcome, MediaBound);
