//! Evidence for one successful payload authentication on a particular key ring.
//! Enrollment resolves these ring-relative slots to deployment-scoped key IDs.
use std::fmt;
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Format {
    V0,
    V1,
    V1Draft,
    V2,
    V3,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    Current,
    Previous,
}

/// Cloud identities name logical resources. Their internal versions require
/// independent evidence. AWS supplies its canonical key ARN in the response.
#[derive(PartialEq, Eq)]
pub(crate) enum LogicalKey {
    Local,
    #[cfg(any(test, feature = "aws-kms"))]
    Aws(String),
    #[cfg(any(test, feature = "gcp-kms"))]
    Gcp(String),
}

#[derive(PartialEq, Eq)]
pub(crate) enum Route {
    Direct(Slot),
    Primary(Slot, LogicalKey),
    Fallback(Slot, LogicalKey),
    Unknown,
}

impl fmt::Debug for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Route([REDACTED])")
    }
}

pub(crate) struct Authenticated {
    plaintext: Zeroizing<Vec<u8>>,
    evidence: Evidence,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "The census consumes the authenticated selector and route"
    )
)]
pub(crate) struct Evidence {
    pub(crate) format: Format,
    pub(crate) selector: Option<u8>,
    pub(crate) route: Route,
}

impl fmt::Debug for Authenticated {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Authenticated")
            .field("format", &self.evidence.format)
            .finish_non_exhaustive()
    }
}

impl Authenticated {
    pub(crate) fn into_parts(self) -> (Zeroizing<Vec<u8>>, Evidence) {
        (self.plaintext, self.evidence)
    }

    pub(in crate::crypto) fn direct(
        plaintext: Zeroizing<Vec<u8>>,
        format: Format,
        selector: Option<u8>,
        slot: Slot,
    ) -> Self {
        Self {
            plaintext,
            evidence: Evidence {
                format,
                selector,
                route: Route::Direct(slot),
            },
        }
    }

    pub(in crate::crypto) fn envelope(
        plaintext: Zeroizing<Vec<u8>>,
        format: Format,
        selector: u8,
        key: Option<(Slot, LogicalKey)>,
        fallback: bool,
    ) -> Self {
        let route = match (key, fallback) {
            (Some((slot, key)), false) => Route::Primary(slot, key),
            (Some((slot, key)), true) => Route::Fallback(slot, key),
            (None, _) => Route::Unknown,
        };
        Self {
            plaintext,
            evidence: Evidence {
                format,
                selector: Some(selector),
                route,
            },
        }
    }
}

/// Select the field's persisted contract before authentication. The census can
/// inspect historical data even when ordinary readers require contextual writes.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Consumed by the next census adapter PR")
)]
pub(crate) enum Binding<'a> {
    Record(Option<u32>, &'a super::RecordContext<'a>),
    Historical(&'a super::historical::HistoricalAad<'a>),
    Oracle(&'a super::oracle::OracleOuter),
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Consumed by the next census adapter PR")
)]
pub(crate) async fn observe(
    keys: &super::EncryptionKeys,
    ciphertext: &[u8],
    binding: Binding<'_>,
) -> crate::errors::AppResult<Authenticated> {
    use crate::crypto::aes::ReadPurpose::Observation;
    use crate::errors::AppError;
    match binding {
        Binding::Record(Some(3), context) => {
            keys.decrypt_record_with_policy(ciphertext, context, false, Observation)
                .await
        }
        Binding::Record(None, _) => keys.open_historical(ciphertext, &[], Observation).await,
        Binding::Record(Some(_), _) => {
            Err(AppError::Internal("Unknown record storage version".into()))
        }
        Binding::Historical(binding) => {
            keys.open_historical(ciphertext, &binding.0, Observation)
                .await
        }
        Binding::Oracle(binding) => {
            let opened = keys
                .open_historical(ciphertext, binding.0.as_bytes(), Observation)
                .await?;
            if opened.evidence.format != Format::V2 {
                return Err(AppError::Internal("Invalid Oracle outer envelope".into()));
            }
            Ok(opened)
        }
    }
}

#[cfg(test)]
mod tests;
