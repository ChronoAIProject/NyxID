use crate::errors::{AppError, AppResult};
use chrono::{DateTime, Duration, Utc};

pub const DEFAULT_NEW_KEY_LIFETIME_DAYS: i64 = 90;

/// Issuance intent only. Stored rows (including legacy nonexpiring keys) and
/// rotations must retain their existing policy instead of applying defaults.
#[derive(Clone, Copy, Debug)]
pub enum NewKeyExpiry {
    Default,
    At(DateTime<Utc>),
    NeverExpires(NonExpiringReason),
}

#[derive(Clone, Copy, Debug)]
pub enum NonExpiringReason {
    UserSelected,
    ManagedNyxbot,
    ManagedConversation,
}

impl From<Option<DateTime<Utc>>> for NewKeyExpiry {
    fn from(value: Option<DateTime<Utc>>) -> Self {
        value.map(Self::At).unwrap_or(Self::Default)
    }
}

impl NewKeyExpiry {
    pub fn from_request(value: Option<&str>, never_expires: bool) -> AppResult<Self> {
        let value = value.filter(|value| !value.is_empty());
        match (value, never_expires) {
            (Some(_), true) => Err(AppError::ValidationError(
                "expires_at cannot be combined with never_expires".into(),
            )),
            (Some(value), false) => Ok(Self::At(parse_expires_at(value)?)),
            (None, true) => Ok(Self::NeverExpires(NonExpiringReason::UserSelected)),
            (None, false) => Ok(Self::Default),
        }
    }

    pub fn resolve_for_storage(
        self,
        created_at: DateTime<Utc>,
    ) -> AppResult<Option<DateTime<Utc>>> {
        let expiry = self
            .resolve(created_at)?
            .map(|value| mongodb::bson::DateTime::from_chrono(value).to_chrono());
        if expiry.is_some_and(|value| value <= created_at) {
            return Err(AppError::ValidationError(
                "expires_at must be in the future at stored precision".into(),
            ));
        }
        Ok(expiry)
    }

    pub fn resolve(self, created_at: DateTime<Utc>) -> AppResult<Option<DateTime<Utc>>> {
        let expiry = match self {
            Self::Default => Some(
                mongodb::bson::DateTime::from_chrono(
                    created_at + Duration::days(DEFAULT_NEW_KEY_LIFETIME_DAYS),
                )
                .to_chrono(),
            ),
            Self::At(value) => Some(value),
            Self::NeverExpires(reason) => {
                // Reading the reason keeps each deliberate exemption explicit
                // at its constructor; it is not a stored-row read default.
                let _ = reason;
                None
            }
        };
        if expiry.is_some_and(|value| value <= created_at) {
            return Err(AppError::ValidationError(
                "expires_at must be in the future".into(),
            ));
        }
        Ok(expiry)
    }
}

pub(crate) fn parse_expires_at(s: &str) -> AppResult<DateTime<Utc>> {
    // Try RFC 3339 first
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    // Try date-only (YYYY-MM-DD) -> end of day UTC
    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        && let Some(dt) = date.and_hms_opt(23, 59, 59)
    {
        return Ok(dt.and_utc());
    }
    Err(AppError::ValidationError(
        "Invalid expires_at format. Use RFC 3339 (e.g. 2026-04-01T00:00:00Z) or date-only (e.g. 2026-04-01)".to_string(),
    ))
}

pub(crate) fn resolve_create_allow_all(
    allowed_ids: &[String],
    requested_allow_all: Option<bool>,
    allow_all_field: &str,
    allowed_ids_field: &str,
) -> AppResult<bool> {
    if requested_allow_all == Some(true) && !allowed_ids.is_empty() {
        return Err(AppError::ValidationError(format!(
            "{allow_all_field} cannot be true when {allowed_ids_field} is non-empty"
        )));
    }

    Ok(requested_allow_all.unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_lifetime_is_finite_but_nonexpiry_requires_explicit_intent() {
        let now = DateTime::parse_from_rfc3339("2026-10-19T00:00:00.123456Z")
            .unwrap()
            .to_utc();
        for value in [None, Some("")] {
            assert_eq!(
                NewKeyExpiry::from_request(value, false)
                    .unwrap()
                    .resolve(now)
                    .unwrap(),
                Some(
                    DateTime::parse_from_rfc3339("2027-01-17T00:00:00.123Z")
                        .unwrap()
                        .to_utc()
                )
            );
            assert_eq!(
                NewKeyExpiry::from_request(value, true)
                    .unwrap()
                    .resolve(now)
                    .unwrap(),
                None
            );
        }
        assert!(NewKeyExpiry::from_request(Some("2099-01-01"), true).is_err());
        assert!(
            NewKeyExpiry::from_request(Some("2000-01-01"), false)
                .unwrap()
                .resolve(now)
                .is_err()
        );
    }

    #[test]
    fn stored_precision_cannot_round_a_future_timestamp_into_expired_policy() {
        let now = DateTime::parse_from_rfc3339("2026-10-19T00:00:00.123400Z")
            .unwrap()
            .to_utc();
        let too_close = now + chrono::Duration::microseconds(100);
        assert!(NewKeyExpiry::At(too_close).resolve(now).is_ok());
        assert!(
            NewKeyExpiry::At(too_close)
                .resolve_for_storage(now)
                .is_err()
        );
        assert_eq!(
            NewKeyExpiry::At(now + Duration::seconds(1))
                .resolve_for_storage(now)
                .unwrap(),
            Some(
                DateTime::parse_from_rfc3339("2026-10-19T00:00:01.123Z")
                    .unwrap()
                    .to_utc()
            )
        );
    }

    #[test]
    fn empty_or_selected_resource_lists_do_not_implicitly_grant_all() {
        for ids in [vec![], vec!["resource".into()]] {
            assert!(!resolve_create_allow_all(&ids, None, "all", "ids").unwrap());
            assert!(!resolve_create_allow_all(&ids, Some(false), "all", "ids").unwrap());
        }
        assert!(resolve_create_allow_all(&[], Some(true), "all", "ids").unwrap());
        assert!(resolve_create_allow_all(&["resource".into()], Some(true), "all", "ids").is_err());
    }
}
