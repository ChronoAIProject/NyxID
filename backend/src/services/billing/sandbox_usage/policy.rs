use crate::{
    errors::{AppError, AppResult},
    models::sandbox_usage_rates::{DimensionPolicy, Selection, UsageScope},
};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

pub(super) fn invalid(message: &str) -> AppError {
    AppError::ValidationError(message.into())
}
pub(super) fn unavailable(message: &str) -> AppError {
    AppError::BillingProviderUnavailable(message.into())
}

pub fn effective_minute(requested: DateTime<Utc>, now: DateTime<Utc>) -> AppResult<DateTime<Utc>> {
    let earliest = now
        .checked_add_signed(chrono::Duration::seconds(
            super::super::sandbox_rates::MIN_NOTICE_SECONDS,
        ))
        .ok_or_else(|| invalid("Publication time overflow"))?;
    if requested < earliest || requested.timestamp() < 0 {
        return Err(invalid(
            "Usage policies and tariffs require at least 301 seconds notice",
        ));
    }
    let seconds = requested
        .timestamp()
        .checked_add(if requested.timestamp_subsec_nanos() == 0 {
            0
        } else {
            1
        })
        .and_then(|v| v.checked_add(59))
        .map(|v| v / 60 * 60)
        .ok_or_else(|| invalid("Publication time overflow"))?;
    DateTime::from_timestamp(seconds, 0).ok_or_else(|| invalid("Publication time overflow"))
}

pub(super) fn text(value: &str) -> AppResult<()> {
    if value.is_empty()
        || value.len() > 256
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(invalid("Invalid usage scope identifier"));
    }
    Ok(())
}

pub fn scope_key(scope: &UsageScope) -> AppResult<String> {
    for value in [
        &scope.producer_namespace,
        &scope.owner_id,
        &scope.sandbox_id,
    ] {
        text(value)?;
    }
    let epoch =
        uuid::Uuid::parse_str(&scope.source_epoch).map_err(|_| invalid("Invalid source epoch"))?;
    if epoch.get_version() != Some(uuid::Version::Random)
        || epoch.get_variant() != uuid::Variant::RFC4122
        || epoch.to_string() != scope.source_epoch
    {
        return Err(invalid("Invalid source epoch"));
    }
    let bytes = serde_json::to_vec(scope).map_err(|_| invalid("Invalid usage scope"))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub fn shadow() -> DimensionPolicy {
    DimensionPolicy {
        cpu: Selection::Shadow,
        memory: Selection::Shadow,
        disk: Selection::Shadow,
    }
}

pub(super) fn minute(at: DateTime<Utc>) -> AppResult<()> {
    if at.timestamp() < 0 || at.timestamp_subsec_nanos() != 0 || at.timestamp() % 60 != 0 {
        return Err(invalid("Usage boundary must be a UTC minute"));
    }
    Ok(())
}
